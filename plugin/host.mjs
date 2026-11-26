import { spawn } from "node:child_process";
import path from "node:path";
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

  async function request(op, payload) {
    if (stopping || child.exitCode !== null || child.signalCode !== null ||
        !child.stdin.writable) {
      throw new Error("LitzSim core is unavailable.");
    }
    let timer;
    try {
      const line = await new Promise((resolve, reject) => {
        pending = { resolve, reject };
        timer = setTimeout(() => reject(new Error("LitzSim core did not respond.")), 3000);
        child.stdin.write(JSON.stringify({ ...payload, op }) + "\n");
      });
      const response = JSON.parse(line);
      if (typeof response?.ok !== "boolean" ||
          (op === "ping" && (response.ok !== true || typeof response.version !== "string" || !response.version))) {
        throw new Error("Invalid LitzSim core response.");
      }
      return response;
    } catch (error) {
      // Without request IDs, dispose before a late response can reach another call.
      await dispose();
      throw error;
    } finally {
      clearTimeout(timer);
      pending = undefined;
    }
  }

  function invoke(op, payload) {
    const task = queue.then(() => request(op, payload));
    queue = task.catch(() => {});
    return task;
  }

  try {
    version = (await request("ping")).version;
  } catch {
    await dispose();
  }

  return {
    status: () => ({
      available: version !== undefined,
      message: version ? `LitzSim core ${version} ready` : "LitzSim core unavailable",
    }),
    operations: {
      ping: () => invoke("ping"),
      new: () => invoke("new"),
      inspect: payload => invoke("inspect", payload),
      layout: payload => invoke("layout", payload),
      apply: payload => invoke("apply", payload),
      undo: payload => invoke("undo", payload),
      save: async ({ folder, name, ...payload }) => {
        if (folder === undefined) return invoke("save", payload);
        if (typeof name !== "string" || !name || path.basename(name) !== name) {
          return { ok: false, error: { code: "invalidRequest", message: "Expected a file name without folders." } };
        }
        // New circuits are saved without an expected revision, which refuses existing files.
        const file = path.join(folder, name);
        const response = await invoke("save", { ...payload, file });
        return response.ok ? { ...response, file } : response;
      },
      start: payload => invoke("start", payload),
      status: payload => invoke("status", payload),
      stop: payload => invoke("stop", payload),
      wave: payload => invoke("wave", payload),
    },
    dispose: () => dispose(),
  };
}
