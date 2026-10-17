"""配布zipの本体の取り違え(egui版をTauri版として同梱)を止める検査。"""
import importlib.util
import struct
from pathlib import Path

import pytest

_spec = importlib.util.spec_from_file_location(
    'rust_artifact', Path(__file__).resolve().parents[1] / 'tools' / 'rust_artifact.py')
rust_artifact = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rust_artifact)


def fake_pe(sections, payload=b''):
    """最小のPE。セクション名の並びだけを持つ。"""
    pe = 0x80
    head = bytearray(pe) + b'PE\0\0' + struct.pack('<HHIIIHH', 0x8664, len(sections), 0, 0, 0, 0xF0, 0x22)
    head[0:2] = b'MZ'
    struct.pack_into('<I', head, 0x3C, pe)
    table = b''.join(name.encode().ljust(8, b'\0') + bytes(32) for name in sections)
    return bytes(head) + bytes(0xF0) + table + payload


def test_tauri_binary_with_resources_is_accepted():
    data = fake_pe(['.text', '.rsrc'], b'src/hashi-desktop-tauri/main.rs')
    rust_artifact.check_gui_binary('Hashi.exe', data)


def test_egui_binary_renamed_as_tauri_is_rejected():
    data = fake_pe(['.text', '.rsrc'], b'hashi-desktop-native/src/main.rs')
    with pytest.raises(ValueError, match='Tauri版ではありません'):
        rust_artifact.check_gui_binary('Hashi.exe', data)


def test_binary_with_both_markers_is_rejected():
    data = fake_pe(['.text', '.rsrc'], b'hashi-desktop-tauri hashi-desktop-native')
    with pytest.raises(ValueError):
        rust_artifact.check_gui_binary('Hashi.exe', data)


def test_egui_file_name_is_rejected():
    data = fake_pe(['.text', '.rsrc'], b'hashi-desktop-tauri')
    with pytest.raises(ValueError, match='Hashi-RustPreview'):
        rust_artifact.check_gui_binary('Hashi-RustPreview.exe', data)


def test_missing_icon_resource_is_rejected():
    data = fake_pe(['.text', '.data'], b'hashi-desktop-tauri')
    with pytest.raises(ValueError, match='rsrc'):
        rust_artifact.check_gui_binary('Hashi.exe', data)


def test_non_pe_is_rejected():
    with pytest.raises(ValueError, match='PE'):
        rust_artifact.check_gui_binary('Hashi.exe', b'\x7fELF hashi-desktop-tauri')
