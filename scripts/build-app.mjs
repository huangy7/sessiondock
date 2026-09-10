import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDir, "..");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    stdio: "inherit",
    shell: false,
  });

  if (result.error) {
    throw result.error;
  }

  if (typeof result.status === "number" && result.status !== 0) {
    process.exit(result.status);
  }
}

function runNodeScript(scriptPath, args = []) {
  run(process.execPath, [scriptPath, ...args]);
}

if (process.env.CLAUDIA_SKIP_PROXY_BUILD !== "1") {
  runNodeScript(resolve(projectRoot, "scripts", "build-proxy.mjs"));
}

runNodeScript(resolve(projectRoot, "node_modules", "vue-tsc", "bin", "vue-tsc.js"), ["--noEmit"]);
runNodeScript(resolve(projectRoot, "node_modules", "vite", "bin", "vite.js"), ["build"]);
