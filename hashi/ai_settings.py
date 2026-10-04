"""AI接続を選び、モデル取得・接続確認・保存を個別に行う。"""
import hashlib
import threading

from PySide6.QtCore import QThread, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLineEdit,
    QPushButton,
    QVBoxLayout,
)

from . import style
from .ai_api import COMMANDCODE_BASE, ApiProvider
from .ai_http import normalize_api_base


class ModelSelector(QComboBox):
    def __init__(self):
        super().__init__()
        self.setEditable(True)
        self.lineEdit().setPlaceholderText("一覧から選ぶか、モデル名を入力")

    def text(self):
        return self.currentText()

    def setText(self, text):
        self.setEditText(text)


class ConnectionWorker(QThread):
    succeeded = Signal(object)
    failed = Signal(str)

    def __init__(self, action, parent):
        super().__init__(parent)
        self.action, self.cancel = action, threading.Event()

    def run(self):
        try:
            if self.cancel.is_set():
                return
            result = self.action(self.cancel)
            if not self.cancel.is_set():
                self.succeeded.emit(result)
        except (ValueError, RuntimeError) as exc:
            if not self.cancel.is_set():
                self.failed.emit(str(exc))
        except Exception:
            if not self.cancel.is_set():
                self.failed.emit("処理に失敗しました。接続先と入力内容を確認してください。")


class AiSettingsDialog(QDialog):
    def __init__(self, settings, secrets, parent=None):
        super().__init__(parent)
        self.settings, self.secrets, self.provider = settings, secrets, None
        self.worker, self._reject_pending = None, False
        self._accept_ready = False
        self.setWindowTitle("AIに接続する")
        self.resize(style.DIALOG_L, 520)
        layout = QVBoxLayout(self)
        layout.addWidget(style.plain_label("ChatGPTプランを使う"))
        chatgpt = QPushButton("ChatGPTでログイン…")
        chatgpt.clicked.connect(self._chatgpt)
        layout.addWidget(chatgpt)
        layout.addWidget(style.plain_label("APIキーを使う"))
        self.kind, self.model = QComboBox(), ModelSelector()
        self.base, self.key = QLineEdit(), QLineEdit()
        for label, kind in (("OpenAI", "openai"), ("Anthropic", "anthropic"),
                            ("その他の互換API", "compatible"), ("Command Code", "commandcode")):
            self.kind.addItem(label, kind)
        self.protocol = QComboBox()
        for label, value in (("自動選択", "auto"), ("Chat Completions", "chat"),
                             ("Responses", "responses"), ("Anthropic Messages", "messages")):
            self.protocol.addItem(label, value)
        self.key.setEchoMode(QLineEdit.Password)
        self.key.setPlaceholderText("この接続先のAPIキーを貼り付け")
        form = QFormLayout()
        for label, widget in (("接続先", self.kind), ("APIのURL", self.base),
                              ("APIキー", self.key), ("モデル", self.model), ("API形式", self.protocol)):
            form.addRow(label, widget)
        layout.addLayout(form)
        row = QHBoxLayout()
        self.fetch_models = QPushButton("モデル一覧を取得")
        self.fetch_models.clicked.connect(self._models)
        self.check_connection = QPushButton("接続を確認")
        self.check_connection.clicked.connect(self._test)
        row.addWidget(self.fetch_models)
        row.addWidget(self.check_connection)
        layout.addLayout(row)
        self.use_tools = QCheckBox("AIから端末ツールを使う")
        self.use_tools.setToolTip("操作する端末と実行許可は、会話画面で別に選びます")
        layout.addWidget(self.use_tools)
        self.persist = QCheckBox("APIキーをこのPCに保存する")
        layout.addWidget(self.persist)
        self.forget = QPushButton("この接続先の保存キーを削除")
        self.forget.clicked.connect(self._forget)
        layout.addWidget(self.forget)
        self.status = style.plain_label("接続先を選び、キーを入力してモデルを選んでください。")
        self.status.setWordWrap(True)
        layout.addWidget(self.status)
        layout.addWidget(style.plain_label("接続確認は短い回答を取得します。API利用料は提供元の料金に従います。"))
        self.extra_layout = layout
        self.buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        self.buttons.button(QDialogButtonBox.Ok).setText("この接続を使う")
        self.buttons.button(QDialogButtonBox.Cancel).setText("キャンセル")
        self.buttons.accepted.connect(self._accept)
        self.buttons.rejected.connect(self.reject)
        layout.addWidget(self.buttons)
        profiles = settings.get("ai_api_profiles")
        self.profiles = dict(profiles) if isinstance(profiles, dict) else {}
        self.kind.setCurrentIndex(max(0, self.kind.findData(settings.get("ai_api_kind") or "openai")))
        self.controls = [self.kind, self.model, self.base, self.key, self.protocol,
                         self.use_tools, self.persist, self.forget, self.fetch_models,
                         self.check_connection, chatgpt, self.buttons.button(QDialogButtonBox.Ok)]
        self._load()
        self.kind.currentIndexChanged.connect(self._load)
        self.base.textEdited.connect(self._base_changed)

    def _base_changed(self, _text):
        self.key.clear()
        self.persist.setChecked(False)

    def key_id(self):
        return "api:" + self.kind.currentData() + ":" + hashlib.sha256(
            normalize_api_base(self.base.text()).encode()).hexdigest()

    def _chatgpt(self):
        from .chatgpt_dialog import ChatGptDialog
        from .chatgpt_oauth import ChatGptAccounts
        dialog = ChatGptDialog(ChatGptAccounts(self.secrets), self)
        if dialog.exec() == QDialog.Accepted:
            self.provider = dialog.provider
            self.accept()

    def _load(self):
        if self.worker is not None:
            return
        kind = self.kind.currentData()
        profile = self.profiles.get(kind, {})
        if not isinstance(profile, dict):
            profile = {}
        defaults = {"openai": "https://api.openai.com/v1", "anthropic": "https://api.anthropic.com/v1",
                    "commandcode": COMMANDCODE_BASE, "compatible": "http://127.0.0.1:1234/v1"}
        self.base.setText(profile.get("base") or defaults[kind])
        self.base.setReadOnly(kind != "compatible")
        self.model.clear()
        self.model.setText(profile.get("model", ""))
        self.protocol.setCurrentIndex(max(0, self.protocol.findData(profile.get("protocol", "auto"))))
        self.protocol.setEnabled(kind in {"commandcode", "compatible"})
        self.use_tools.setChecked(profile.get("use_tools", True))
        self._catalog = {}
        self.key.clear()
        self.persist.setChecked(False)
        try:
            key_id = self.key_id()
        except ValueError as exc:
            self.status.setText(str(exc))
            return
        def read_key(_cancel):
            try:
                value = self.secrets.get(key_id)
                if value is not None and not isinstance(value, str):
                    raise ValueError("保存キーの形式が不正")
                return value or ""
            except Exception:
                raise RuntimeError("保存キーを読み込めませんでした。キーを再入力してください。") from None
        def loaded(value):
            if not self._reject_pending and self.key_id() == key_id and not self.key.text():
                self.key.setText(value)
                self.persist.setChecked(bool(value))
                self.status.setText("モデル一覧を取得するか、利用可能なモデル名を入力してください。")
        self._start(read_key, loaded, "このPCの保存キーを読み込んでいます…")

    def _provider(self, require_model=True):
        kind, base = self.kind.currentData(), normalize_api_base(self.base.text())
        if not self.key.text().strip() and not (kind == "compatible" and base.startswith("http://")):
            raise ValueError("APIキーを入力してください。")
        model = self.model.text().strip()
        if require_model and not model:
            raise ValueError("モデル一覧から選ぶか、モデル名を入力してください。")
        protocol = self.protocol.currentData() if kind in {"compatible", "commandcode"} else "auto"
        endpoints = self._catalog.get(model, {}).get("supported_endpoints", [])
        if protocol == "auto" and isinstance(endpoints, list) and endpoints:
            protocol = next((p for suffix, p in (("chat/completions", "chat"), ("messages", "messages"),
                                                  ("responses", "responses"))
                             if any(isinstance(e, str) and e.endswith(suffix) for e in endpoints)), "auto")
        return ApiProvider(kind, model or "model-list", self.key.text().strip(), base_url=base,
                           protocol=protocol, use_tools=self.use_tools.isChecked())

    def _start(self, action, receiver, message):
        if self.worker is not None:
            return
        self.status.setText(message)
        worker = ConnectionWorker(action, self)
        self.worker = worker
        for control in self.controls:
            control.setEnabled(False)
        worker.succeeded.connect(receiver)
        worker.failed.connect(self.status.setText)
        worker.finished.connect(self._finished)
        worker.start()

    def _finished(self):
        # Pythonの接続コールバックを含むQThreadは、完了通知中に解放せず
        # 親ダイアログの寿命まで保持する（PySideの破棄競合を避ける）。
        self.worker = None
        for control in self.controls:
            control.setEnabled(True)
        self.protocol.setEnabled(self.kind.currentData() in {"compatible", "commandcode"})
        if self._reject_pending:
            super().reject()
        elif self._accept_ready:
            self.accept()

    def _models(self):
        try:
            provider = self._provider(False)
        except ValueError as exc:
            self.status.setText(str(exc))
            return
        self._start(provider.models, self._got_models, "モデル一覧を取得しています…")

    def _got_models(self, models):
        previous = self.model.text()
        self._catalog = {m["id"]: m for m in models}
        self.model.clear()
        self.model.addItems(list(self._catalog))
        if previous:
            self.model.setText(previous)
        self.status.setText(f"{len(models)}個のモデルを取得しました。モデルを選んでください。" if models
                            else "一覧にモデルがありません。提供元のモデル名を手動で入力してください。")

    def _test(self):
        try:
            provider = self._provider()
        except ValueError as exc:
            self.status.setText(str(exc))
            return
        def check(cancel):
            provider.use_tools = False
            provider.respond([{"role": "user", "content": "OKとだけ答えてください。"}],
                             "接続確認です。端末操作をせず短く回答してください。", [], cancel, lambda _text: None)
        self._start(check, lambda _: self.status.setText("接続できました。「この接続を使う」で会話へ戻れます。"),
                    "モデルからの応答を確認しています…")

    def _forget(self):
        try:
            key_id = self.key_id()
        except ValueError as exc:
            self.status.setText(str(exc))
            return
        def forget(_cancel):
            try:
                self.secrets.delete(key_id)
            except Exception:
                raise RuntimeError("保存キーを削除できませんでした。PCの資格情報ストアを確認してください。") from None
        def done(_result):
            self.key.clear()
            self.persist.setChecked(False)
            self.status.setText("この接続先の保存キーを削除しました。")
        self._start(forget, done, "保存キーを削除しています…")

    def _accept(self):
        try:
            provider, key_id = self._provider(), self.key_id()
        except ValueError as exc:
            self.status.setText(str(exc))
            return
        persist = self.persist.isChecked()
        profile = {"base": provider.base, "model": provider.model, "protocol": provider.protocol,
                   "use_tools": self.use_tools.isChecked()}
        kind = self.kind.currentData()
        def save(_cancel):
            try:
                if persist:
                    self.secrets.set(key_id, provider._key)
                else:
                    self.secrets.delete(key_id)
            except Exception:
                raise RuntimeError("APIキーの保存・削除に失敗しました。PCの資格情報ストアを確認して再試行してください。") from None
            try:
                self.settings.set("ai_api_profiles", {**self.profiles, kind: profile})
                self.settings.set("ai_api_kind", kind)
            except Exception:
                raise RuntimeError("接続設定を保存できませんでした。設定フォルダの書込み権限を確認してください。") from None
            return provider
        def done(value):
            if not self._reject_pending:
                self.provider = value
                self._accept_ready = True
        self._start(save, done, "接続設定を保存しています…")

    def reject(self):
        if self.worker is not None:
            self._reject_pending = True
            self.worker.cancel.set()
            self.status.setText("処理の終了を待っています…")
            return
        super().reject()

    def closeEvent(self, event):
        if self.worker is not None:
            self.reject()
            event.ignore()
        else:
            super().closeEvent(event)
