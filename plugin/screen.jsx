import { Editor } from "./editor.jsx";

// The only layer that touches Litzio: file pickers, confirmation, UI parts and
// the plugin invoke channel. A standalone shell can supply the same adapter.
export default function Screen({ React, ui, invoke, pickPath, confirm }) {
  const adapter = React.useMemo(() => ({
    async call(operation, input = {}) {
      const response = await invoke(operation, input);
      if (response?.ok !== true) {
        const error = new Error(response?.error?.message ?? "LitzSim core did not complete the request.");
        error.code = response?.error?.code;
        throw error;
      }
      return response;
    },
    async pickCircuit() {
      return (await pickPath({ kind: "file", extensions: [".litzsim"] }))?.path ?? null;
    },
    async pickFolder() {
      return (await pickPath({ kind: "folder" }))?.path ?? null;
    },
    confirm,
  }), [invoke, pickPath, confirm]);
  return <Editor ui={ui} adapter={adapter} />;
}
