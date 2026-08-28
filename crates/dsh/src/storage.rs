use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde_json::{Value, json};
use tokn_dsh_protocol::{DshSessionItem, DshSessionLine};

/// Snapshot the byte length, so a concurrently appended session cannot make a
/// historical read follow forever. Truncated/corrupt frames are reported, not
/// repaired; the harness owns its storage.
pub(crate) fn reader(path: &Path) -> Result<Box<dyn BufRead>, String> {
  let file = File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
  let len = file.metadata().map_err(|err| err.to_string())?.len();
  let input = file.take(len);
  if path.to_str().is_some_and(|name| name.ends_with(".jsonl.zstd")) {
    let decoder =
      zstd::stream::read::Decoder::new(input).map_err(|err| format!("failed to decode {}: {err}", path.display()))?;
    // Decoder reads all concatenated frames by default, as written by DSH.
    Ok(Box::new(BufReader::new(decoder)))
  } else {
    Ok(Box::new(BufReader::new(input)))
  }
}

pub(crate) fn parse(line: &str, path: &Path, index: usize) -> Result<DshSessionLine, String> {
  serde_json::from_str(line).map_err(|err| format!("invalid dsh JSON at {}:{index}: {err}", path.display()))
}

/// Expand the physical row representation before normalization. Unknown
/// ordinary events pass through, but a malformed packed run must never silently
/// lose its members. Validate before emitting any of them.
pub(crate) fn expand(line: DshSessionLine) -> Result<Vec<DshSessionLine>, String> {
  let native = line.native();
  let tag = native.get("type").and_then(Value::as_str).unwrap_or("");
  if !matches!(tag, "text-chunks" | "reasoning-chunks" | "tool-call-chunks") {
    return Ok(vec![line]);
  }
  let malformed = || format!("malformed {tag} storage row");
  let (seq, mut time, turn, step, index, dt, members, call) = match line.item() {
    DshSessionItem::TextChunks(row) | DshSessionItem::ReasoningChunks(row) => {
      if !row.extra.is_empty() || !row.data.extra.is_empty() {
        return Err(malformed());
      }
      (
        row.seq0,
        row.time0,
        row.data.turn,
        row.data.step,
        row.data.index,
        &row.data.dt,
        &row.data.texts,
        None,
      )
    }
    DshSessionItem::ToolCallChunks(row) => {
      if !row.extra.is_empty() || !row.data.extra.is_empty() || native["data"].get("name").is_some_and(Value::is_null) {
        return Err(malformed());
      }
      (
        row.seq0,
        row.time0,
        row.data.turn,
        row.data.step,
        row.data.index,
        &row.data.dt,
        &row.data.args,
        Some((&row.data.id, &row.data.name)),
      )
    }
    _ => return Err(malformed()),
  };
  const MAX_SAFE: i64 = 9_007_199_254_740_991;
  if members.is_empty()
    || dt.len() != members.len() - 1
    || seq
      .checked_add(members.len() as u64 - 1)
      .is_none_or(|end| end > MAX_SAFE as u64)
    || !(-MAX_SAFE..=MAX_SAFE).contains(&time)
  {
    return Err(malformed());
  }
  let mut events = Vec::with_capacity(members.len());
  for (offset, member) in members.iter().enumerate() {
    if offset > 0 {
      let gap = dt[offset - 1];
      if !(-MAX_SAFE..=MAX_SAFE).contains(&gap) {
        return Err(malformed());
      }
      time = time
        .checked_add(gap)
        .filter(|time| (-MAX_SAFE..=MAX_SAFE).contains(time))
        .ok_or_else(malformed)?;
    }
    let chunk = if let Some((id, name)) = call {
      let mut chunk = json!({"type": "tool-call-delta", "index": index, "id": id, "argumentsDelta": member});
      if let Some(name) = name {
        chunk["name"] = json!(name);
      }
      chunk
    } else {
      json!({"type": if tag == "text-chunks" { "text-delta" } else { "reasoning-delta" }, "index": index, "text": member})
    };
    events.push(
      serde_json::from_value(json!({
        "type": "assistant/chunk", "seq": seq + offset as u64, "time": time,
        "data": { "turn": turn, "step": step, "chunk": chunk }
      }))
      .map_err(|err| format!("invalid expanded chunk: {err}"))?,
    );
  }
  Ok(events)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn expands_each_packed_kind_with_exact_member_times_and_call_identity() {
    for (tag, delta_tag, payload_key) in [
      ("text-chunks", "text-delta", "texts"),
      ("reasoning-chunks", "reasoning-delta", "texts"),
      ("tool-call-chunks", "tool-call-delta", "args"),
    ] {
      for named in [false, true] {
        let mut data = json!({"turn": 2, "step": 3, "index": 4, "dt": [2, -3]});
        data[payload_key] = json!(["a", "b", "c"]);
        if tag == "tool-call-chunks" {
          data["id"] = json!("call");
          if named {
            data["name"] = json!("read");
          }
        }
        let row = serde_json::from_value(json!({"type": tag, "seq0": 7, "time0": 10, "data": data})).unwrap();
        let events = expand(row).unwrap();
        for (index, (text, time)) in [("a", 10), ("b", 12), ("c", 9)].iter().enumerate() {
          let mut chunk = json!({"type": delta_tag, "index": 4});
          if tag == "tool-call-chunks" {
            chunk["id"] = json!("call");
            chunk["argumentsDelta"] = json!(text);
            if named {
              chunk["name"] = json!("read");
            }
          } else {
            chunk["text"] = json!(text);
          }
          assert_eq!(
            events[index].native(),
            &json!({"type":"assistant/chunk","seq":7+index,"time":time,
            "data":{"turn":2,"step":3,"chunk":chunk}})
          );
        }
      }
    }
  }

  #[test]
  fn rejects_invalid_arity_overflow_and_unrecognized_packed_fields() {
    let valid = json!({"type":"text-chunks","seq0":0,"time0":0,
      "data":{"turn":1,"step":1,"index":0,"dt":[1],"texts":["a","b"]}});
    for (pointer, value) in [
      ("/data/dt", json!([])),
      ("/data/texts", json!([])),
      ("/seq0", json!(9_007_199_254_740_991_u64)),
      ("/time0", json!(9_007_199_254_740_991_i64)),
      ("/data/dt", json!([9_007_199_254_740_992_i64])),
      ("/data/texts", json!([1, 2])),
    ] {
      let mut native = valid.clone();
      *native.pointer_mut(pointer).unwrap() = value;
      assert!(expand(serde_json::from_value(native).unwrap()).is_err(), "{pointer}");
    }
    let mut native = valid;
    native["future"] = json!(true);
    assert!(expand(serde_json::from_value(native).unwrap()).is_err());
  }
}
