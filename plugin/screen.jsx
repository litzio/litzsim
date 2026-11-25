async function checkCore(invoke) {
  const start = performance.now();
  const { version } = await invoke("ping", {});
  return { version, roundTripMs: performance.now() - start };
}

export default function Screen({ React, ui, invoke }) {
  const [result, setResult] = React.useState(null);
  const [error, setError] = React.useState("");
  const [busy, setBusy] = React.useState(false);

  async function onCheck() {
    setBusy(true);
    setError("");
    setResult(null);
    try {
      setResult(await checkCore(invoke));
    } catch (error) {
      setError(error.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <ui.ScreenLayout actions={
      <ui.Button variant="default" disabled={busy} onClick={onCheck}>코어 확인</ui.Button>
    }>
      <p role="status">
        {busy && "코어 확인 중…"}
        {result && `코어 버전 ${result.version} · 왕복 ${result.roundTripMs.toFixed(1)} ms`}
      </p>
      {error && <ui.Notice tone="error">{error}</ui.Notice>}
    </ui.ScreenLayout>
  );
}
