// 画面の読み込みに失敗したとき、真っ白のままにせず原因を本文へ出す（アプリ本体より先に読む）。
(function () {
  var errors = [];
  function show() {
    var root = document.getElementById('root');
    if (root && root.childNodes.length > 0) return;
    var box = document.createElement('pre');
    box.style.cssText = 'margin:16px;white-space:pre-wrap;font:13px monospace;color:#b00020';
    box.textContent = '画面を開始できません。\n' + (errors.join('\n') || '（エラーの記録はありません）');
    document.body.appendChild(box);
  }
  window.addEventListener('error', function (e) { errors.push(e.message); });
  window.addEventListener('unhandledrejection', function (e) { errors.push(String(e.reason)); });
  window.addEventListener('securitypolicyviolation', function (e) { errors.push('CSP: ' + e.violatedDirective + ' ' + e.blockedURI); });
  window.setTimeout(show, 4000);
})();
