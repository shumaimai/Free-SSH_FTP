import { createRoot } from 'react-dom/client';
import { loadApi } from './api';
import { App } from './App';
import './styles.css';

// 画面が真っ白のまま原因が分からない事態を避け、起動時の例外は本文へ出す。
function showFatal(message: string) {
  const box = document.createElement('pre');
  box.style.cssText = 'margin:16px;white-space:pre-wrap;font:13px monospace;color:#b00020';
  box.textContent = `画面を開始できません: ${message}`;
  document.body.appendChild(box);
}
window.addEventListener('error', (e) => showFatal(e.message));
window.addEventListener('unhandledrejection', (e) => showFatal(String(e.reason)));

// StrictModeは開発時にeffectを2回走らせ、端末/接続を二重に開くため使わない。
loadApi()
  .then((api) => createRoot(document.getElementById('root')!).render(<App api={api} />))
  .catch((error) => showFatal(String(error)));
