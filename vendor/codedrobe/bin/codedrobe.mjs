#!/usr/bin/env node
import { runCli } from "../src/cli.mjs";
import { DIAGNOSTIC_PREFIX, safeDiagnostic } from "../src/diagnostic.mjs";

try {
  await runCli();
} catch (error) {
  console.error(DIAGNOSTIC_PREFIX + JSON.stringify(safeDiagnostic(error)));
  process.exitCode = 1;
}
