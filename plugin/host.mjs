import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

export async function activate() {
  const executable = fileURLToPath(new URL("./bin/litzsim.exe", import.meta.url));
  const child = spawn(executable, ["serve"], {
    stdio: ["pipe", "pipe", "ignore"],
    windowsHide: true,
  });
  const closed = new Promise(resolve => child.once("close", resolve));
  let pending;
  let version;
  let stopping;
  let queue = Promise.resolve();

  function fail(error) {
    version = undefined;
    pending?.reject(error);
  }

  child.on("error", fail);
  child.stdin.on("error", fail);
  child.stdout.on("error", fail);
  child.on("close", () => fail(new Error("LitzSim core exited.")));
  const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
  lines.on("line", line => pending?.resolve(line));

  function dispose() {
    if (!stopping) {
      fail(new Error("LitzSim core is stopping."));
      const force = setTimeout(() => child.kill("SIGKILL"), 1000);
      stopping = closed.finally(() => clearTimeout(force));
      child.stdin.end();
    }
    return stopping;
  }

  async function requestPing() {
    if (stopping || child.exitCode !== null || child.signalCode !== null ||
        !child.stdin.writable) {
      throw new Error("LitzSim core is unavailable.");
    }
    let timer;
    try {
      const line = await new Promise((resolve, reject) => {
        pending = { resolve, reject };
        timer = setTimeout(() => reject(new Error("LitzSim core did not respond.")), 3000);
        child.stdin.write('{"op":"ping"}\n');
      });
      const response = JSON.parse(line);
      if (response?.ok !== true || typeof response.version !== "string" || !response.version) {
        throw new Error("Invalid LitzSim core response.");
      }
      return response;
    } catch (error) {
      // Without request IDs, a late response must never be used for a later ping.
      await dispose();
      throw error;
    } finally {
      clearTimeout(timer);
      pending = undefined;
    }
  }

  try {
    version = (await requestPing()).version;
  } catch {
    await dispose();
  }

  return {
    status: () => ({
      available: version !== undefined,
      message: version ? `LitzSim core ${version} ready` : "LitzSim core unavailable",
    }),
    operations: {
      ping() {
        const task = queue.then(requestPing);
        queue = task.catch(() => {});
        return task;
      },
    },
    dispose,
  };
}
