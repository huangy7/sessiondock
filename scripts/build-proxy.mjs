import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDir, "..");
const extraArgs = process.argv.slice(2);

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

if (process.platform === "darwin") {
  run("bash", [resolve(projectRoot, "scripts", "mac", "build-proxy.sh"), ...extraArgs]);
} else if (process.platform === "win32") {
  run("cmd.exe", [
    "/d",
    "/c",
    resolve(projectRoot, "scripts", "windows", "build-proxy.bat"),
    ...extraArgs,
  ]);
} else {
  console.error(`Unsupported platform for proxy build: ${process.platform}`);
  process.exit(1);
}
