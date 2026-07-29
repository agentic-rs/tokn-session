import {
  chmod,
  mkdir,
  open,
  rename,
  unlink
} from "node:fs/promises";
import { dirname } from "node:path";

export async function writeFileAtomically(
  path: string,
  contents: string,
  mode = 0o600
): Promise<void> {
  const parent = dirname(path);
  await mkdir(parent, { recursive: true, mode: 0o700 });
  const temporary = `${path}.${process.pid}.${crypto.randomUUID()}.tmp`;
  let created = false;
  try {
    const file = await open(temporary, "wx", mode);
    created = true;
    try {
      await file.writeFile(contents, "utf8");
      await file.sync();
    } finally {
      await file.close();
    }
    await rename(temporary, path);
    created = false;
    if (process.platform !== "win32") {
      await chmod(path, mode);
    }
  } finally {
    if (created) {
      await unlink(temporary).catch(() => {});
    }
  }
}
