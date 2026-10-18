import { createRoot } from 'react-dom/client';
import { loadApi } from './api';
import { App } from './App';
import { ErrorBoundary } from './ErrorBoundary';
import { t } from './i18n';
import { reportClientError } from './report';
import './styles.css';

// 画面が真っ白のまま原因が分からない事態を避け、起動時の例外は本文へ出す。
function showFatal(message: string) {
  const box = document.createElement('pre');
  // 画面の上に重ねて出す（本文の後ろに足すと、高さ100%の画面の下へ隠れて見えない）。
  box.style.cssText =
    'position:fixed;left:16px;right:16px;top:16px;z-index:2147483647;max-height:60vh;overflow:auto;margin:0;padding:12px;white-space:pre-wrap;font:13px monospace;color:#b00020;background:#fff;border:1px solid #b00020;border-radius:6px';
  box.textContent = t('画面を開始できません: {message}', { message });
  document.body.appendChild(box);
  reportClientError(message);
}
window.addEventListener('error', (e) => showFatal(e.message));
window.addEventListener('unhandledrejection', (e) => showFatal(String(e.reason)));

// StrictModeは開発時にeffectを2回走らせ、端末/接続を二重に開くため使わない。
loadApi()
  .then((api) => createRoot(document.getElementById('root')!).render(
      <ErrorBoundary>
        <App api={api} />
      </ErrorBoundary>,
    ))
  .catch((error) => showFatal(String(error)));
