// Rust側メッセージの英訳台帳(src/i18n/rust/*.json)をまとめる。形式は tools/rust_messages.py の説明を参照。
const files = import.meta.glob<Record<string, string | null>>('./rust/*.json', { eager: true, import: 'default' });

export const RUST_MESSAGES: Record<string, string | null> = Object.assign({}, ...Object.values(files));
