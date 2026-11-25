import React from 'react';
import { createRoot } from 'react-dom/client';
import { Editor } from './plugin/screen.jsx';

async function call(name, input) {
  const response = await fetch('/operation/' + name, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error);
  return value.result;
}

function Browser() {
  const [picker, setPicker] = React.useState(null);
  const adapter = {
    call,
    async pickCircuit() {
      const response = await fetch('/files');
      if (!response.ok) throw new Error('Cannot list the fixture directory.');
      const files = await response.json();
      return new Promise(resolve => setPicker({ files, resolve }));
    },
  };
  function select(file) {
    picker.resolve(file);
    setPicker(null);
  }
  return <>
    <Editor React={React} adapter={adapter} />
    {picker && <div role="dialog" aria-modal="true" aria-label="Open circuit"
      style={{ position: 'fixed', top: '15%', left: 'calc(50% - 160px)', width: 320, background: 'white',
        boxShadow: '0 0 0 100vmax #0005', border: '1px solid #cdd7d5', padding: 20 }}>
      <p>Open circuit</p>
      {picker.files.map(file => <p key={file}><button onClick={() => select(file)}>{file}</button></p>)}
      <button onClick={() => select(null)}>Cancel</button>
    </div>}
  </>;
}

createRoot(document.getElementById('root')).render(<Browser />);
