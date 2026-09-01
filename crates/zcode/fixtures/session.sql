create table session (
  id text primary key,
  parent_id text,
  directory text not null,
  title text,
  time_created integer not null,
  time_updated integer not null
);
create table message (
  id text primary key,
  session_id text not null,
  time_created integer not null,
  data text not null,
  sequence integer
);
create table part (
  id text primary key,
  message_id text not null,
  session_id text not null,
  time_created integer not null,
  data text not null,
  sequence integer
);
create table session_entry (
  id text primary key,
  session_id text not null,
  type text not null,
  time_created integer not null,
  data text not null
);

insert into session values (
  'sess_zcode', null, '/tmp/zcode-project', 'ZCode fixture', 1000, 3000
);

insert into session_entry values (
  'entry_model', 'sess_zcode', 'runtime/model_selection', 1050,
  '{"providerId":"zai-coding-plan","modelId":"glm-5","thoughtLevel":"high"}'
);
insert into session_entry values (
  'entry_future', 'sess_zcode', 'runtime/future_entry', 1250,
  '{"answer":42}'
);
insert into session values (
  'sess_child', 'sess_zcode', '/tmp/zcode-project', 'Child fixture', 2000, 2000
);

insert into message values (
  'msg_user',
  'sess_zcode',
  1100,
  '{"role":"user","agent":"zcode-agent","model":{"providerID":"zai-coding-plan","modelID":"glm-5"},"semantics":{"kind":"user_prompt","origin":"real_user","transcriptVisibility":"visible","uiVisibility":"visible","providerVisibility":"visible"}}',
  1
);
insert into part values (
  'part_user', 'msg_user', 'sess_zcode', 1101,
  '{"type":"text","text":"inspect this project","time":{"start":1101,"end":1102}}', 1
);

insert into message values (
  'msg_hidden',
  'sess_zcode',
  1200,
  '{"role":"user","agent":"zcode-agent","source":"todo_reminder","synthetic":true,"semantics":{"kind":"todo_reminder","origin":"agent_runtime","source":"todo_reminder","transcriptVisibility":"hidden","uiVisibility":"hidden","providerVisibility":"visible"}}',
  2
);
insert into part values (
  'part_hidden', 'msg_hidden', 'sess_zcode', 1201,
  '{"type":"text","text":"model-only reminder","synthetic":true}', 1
);

insert into message values (
  'msg_assistant',
  'sess_zcode',
  1300,
  '{"role":"assistant","parentID":"msg_user","agent":"zcode-agent","providerID":"zai-coding-plan","modelID":"glm-5","tokens":{"input":10,"output":5,"reasoning":2,"cache":{"read":3,"write":0},"total":20},"semantics":{"kind":"assistant_response","origin":"agent_runtime","transcriptVisibility":"visible","uiVisibility":"visible","providerVisibility":"visible"}}',
  3
);
insert into part values (
  'part_reasoning', 'msg_assistant', 'sess_zcode', 1301,
  '{"type":"reasoning","text":"checking","metadata":{"anthropic":{"signature":"sig_fixture"}}}', 1
);
insert into part values (
  'part_tool', 'msg_assistant', 'sess_zcode', 1302,
  '{"type":"tool","callID":"call_fixture","tool":"Bash","state":{"status":"completed","input":{"command":"cargo test"},"output":"ok","title":"Run tests","metadata":{"schemaVersion":1}}}', 2
);
insert into part values (
  'part_text', 'msg_assistant', 'sess_zcode', 1303,
  '{"type":"text","text":"done"}', 3
);
insert into part values (
  'part_finish', 'msg_assistant', 'sess_zcode', 1304,
  '{"type":"step-finish","reason":"stop","tokens":{"input":11,"output":6,"reasoning":2,"cache":{"read":4,"write":0},"total":23}}', 4
);
