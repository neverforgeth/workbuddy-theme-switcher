/* global process */

import { existsSync } from "node:fs";
import { delimiter, join } from "node:path";
import { spawnSync } from "node:child_process";

const [command, ...args] = process.argv.slice(2);

if (!command) {
  throw new Error("Expected a command to run.");
}

const cargoHomes = [
  process.env.CARGO_HOME,
  process.env.USERPROFILE ? join(process.env.USERPROFILE, ".cargo") : undefined,
].filter(Boolean);

const cargoDirectory = cargoHomes
  .map((home) => join(home, "bin"))
  .find((directory) => existsSync(join(directory, process.platform === "win32" ? "cargo.exe" : "cargo")));

const env = {
  ...process.env,
  ...(cargoDirectory ? { PATH: `${cargoDirectory}${delimiter}${process.env.PATH ?? ""}` } : {}),
};

const executable = command === "tauri" ? process.execPath : command;
const executableArgs = command === "tauri"
  ? [join(process.cwd(), "node_modules", "@tauri-apps", "cli", "tauri.js"), ...args]
  : args;

const result = spawnSync(executable, executableArgs, {
  cwd: process.cwd(),
  env,
  stdio: "inherit",
  shell: false,
});

process.exitCode = result.status ?? 1;
