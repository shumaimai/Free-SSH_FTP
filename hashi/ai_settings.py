"""AI APIの接続先と秘密を分けて管理する設定。"""
import hashlib

from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QLineEdit,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
)

from . import style
from .ai_api import ApiProvider
from .ai_http import validate_base_url


class AiSettingsDialog(QDialog):
    def __init__(self, settings, secrets, parent=None):
        super().__init__(parent)
        self.settings, self.secrets, self.provider = settings, secrets, None
        self.setWindowTitle("AI接続設定")
        self.resize(style.DIALOG_M, 360)
        layout = QVBoxLayout(self)
        layout.addWidget(style.plain_label("API方式は提供元のAPI料金が適用されます。利用可能なモデル名を指定してください。"))
        self.kind, self.model, self.base, self.key = QComboBox(), QLineEdit(), QLineEdit(), QLineEdit()
        for label, kind in (("OpenAI API", "openai"), ("Anthropic API", "anthropic"), ("OpenAI互換API", "compatible")):
            self.kind.addItem(label, kind)
        self.key.setEchoMode(QLineEdit.Password)
        form = QFormLayout()
        for label, widget in (("方式", self.kind), ("モデル", self.model), ("ベースURL", self.base), ("APIキー", self.key)):
            form.addRow(label, widget)
        layout.addLayout(form)
        self.use_tools = QCheckBox("互換APIで端末ツールを使う（未対応なら外して相談）")
        layout.addWidget(self.use_tools)
        self.persist = QCheckBox("APIキーをこのPCに保存する")
        layout.addWidget(self.persist)
        forget = QPushButton("保存キーを削除")
        forget.clicked.connect(self._forget)
        layout.addWidget(forget)
        self.extra_layout = layout
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)
        self.profiles = settings.get("ai_api_profiles") or {}
        self.kind.setCurrentIndex(max(0, self.kind.findData(settings.get("ai_api_kind") or "openai")))
        self._load()
        self.kind.currentIndexChanged.connect(self._load)
        self.base.textEdited.connect(lambda: self.key.clear())

    def key_id(self):
        return "api:" + self.kind.currentData() + ":" + hashlib.sha256(self.base.text().strip().rstrip("/").encode()).hexdigest()

    def _load(self):
        kind = self.kind.currentData()
        profile = self.profiles.get(kind, {})
        self.base.setText(profile.get("base") or {"openai": "https://api.openai.com/v1",
                         "anthropic": "https://api.anthropic.com/v1", "compatible": "http://127.0.0.1:1234/v1"}[kind])
        self.base.setReadOnly(kind != "compatible")
        self.model.setText(profile.get("model", ""))
        self.use_tools.setVisible(kind == "compatible")
        self.use_tools.setChecked(profile.get("use_tools", True))
        try:
            self.key.setText(self.secrets.get(self.key_id()) or "")
        except Exception:
            self.key.clear()
        self.persist.setChecked(bool(self.key.text()))

    def _forget(self):
        try:
            self.secrets.delete(self.key_id())
            self.key.clear()
            self.persist.setChecked(False)
        except Exception:
            QMessageBox.warning(self, "保存キー", "保存キーを削除できませんでした")

    def _accept(self):
        try:
            kind, base = self.kind.currentData(), validate_base_url(self.base.text().strip())
            if not self.key.text() and kind != "compatible":
                raise ValueError("APIキーを指定してください")
            self.provider = ApiProvider(kind, self.model.text(), self.key.text(), base_url=base,
                                        use_tools=self.use_tools.isChecked())
            if self.persist.isChecked():
                self.secrets.set(self.key_id(), self.key.text())
            else:
                self.secrets.delete(self.key_id())
            self.profiles[kind] = {"base": base, "model": self.model.text().strip(),
                                   "use_tools": self.use_tools.isChecked()}
            self.settings.set("ai_api_profiles", self.profiles)
            self.settings.set("ai_api_kind", kind)
            self.accept()
        except ValueError as exc:
            QMessageBox.warning(self, "AI接続設定", str(exc))
        except Exception:
            QMessageBox.warning(self, "AI接続設定", "設定または認証情報を保存できませんでした")
