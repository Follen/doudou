#!/usr/bin/env node

"use strict";

const { runCli } = require("../src/cli");

runCli(process.argv.slice(2)).catch((error) => {
  const code = error && error.code ? error.code : "UNEXPECTED_ERROR";
  const message = error && error.message ? error.message : String(error);
  const details = error && error.details ? error.details : null;
  process.stdout.write(`${JSON.stringify({ event: "error", code, message, details })}\n`);
  process.exitCode = error && Number.isInteger(error.exitCode) ? error.exitCode : 1;
});
