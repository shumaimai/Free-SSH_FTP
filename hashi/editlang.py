"""内蔵エディタの拡張子レジストリと言語判定の単一ソース。

`filebrowser` の「内蔵エディタで開くか」と `editor` のシンタックスハイライトの
両方がここを参照する。拡張子を増やすときはこのファイルだけ触ればよい。
"""
from __future__ import annotations

import os

# 明らかにテキストとして編集する拡張子。
TEXT_EXTENSIONS = frozenset({
    # 汎用 / ドキュメント
    "txt", "text", "log", "md", "markdown", "mdx", "rst", "adoc", "asciidoc",
    "org", "tex", "bib", "csv", "tsv", "rtf",
    "readme", "license", "licence", "changelog", "authors", "contributing",
    "copying", "notice", "credits", "todo", "bugs", "news", "history",
    # Python
    "py", "pyw", "pyi", "pyx", "pxd", "pxi", "pyd", "ipynb",
    # C / C++ / 系統
    "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "ino", "cu", "cuh",
    # Java / JVM
    "java", "kt", "kts", "scala", "sc", "groovy", "gradle", "clj", "cljs",
    "cljc", "edn", "gradle.kts", "swift",
    # .NET / Windows スクリプト
    "cs", "fs", "fsx", "fsi", "vb", "vbs", "bas", "ps1", "psm1", "psd1",
    "bat", "cmd",
    # Web フロント
    "js", "jsx", "mjs", "cjs", "ts", "tsx", "vue", "svelte", "astro",
    "css", "scss", "sass", "less", "styl", "pcss",
    "html", "htm", "xhtml", "xml", "xsl", "xslt", "svg", "vue", "mustache",
    "hbs", "handlebars", "ejs", "njk", "nunjucks", "liquid", "twig", "pug",
    "jade", "haml", "slim",
    # スクリプト / シェル
    "sh", "bash", "zsh", "fish", "awk", "sed", "rc", "profile",
    "perl", "pl", "pm", "t", "rb", "erb", "gemspec", "rake", "php", "phtml",
    "lua", "luau", "nut", "gd", "as", "gsc", "cs2",
    # Rust / Go / モダン言語
    "rs", "go", "mod", "sum", "nim", "zig", "v", "vala", "d", "di",
    # 関数型
    "hs", "lhs", "ml", "mli", "mll", "mly", "elm", "ex", "exs", "erl", "hrl",
    # データ / 設定
    "json", "jsonc", "json5", "jsonl", "ndjson", "geojson", "yaml", "yml",
    "toml", "ini", "cfg", "conf", "config", "cnf", "properties", "props",
    "env", "dotenv", "settings", "params", "opts", "ruleset",
    "sql", "psql", "mysql", "ddl", "dml", "graphql", "gql", "proto", "thrift",
    "avsc", "avdl", "wsdl", "xsd", "plist", "reg", "inf", "manifest",
    # インフラ / DevOps
    "dockerfile", "containerfile", "tf", "tfvars", "hcl", "nomad", "consul",
    "pp", "puppet", "sls", "ansible", "j2", "jinja", "jinja2",
    "nginx", "vhost", "htaccess", "apache2", "service", "socket", "timer",
    "target", "mount", "network", "slice", "path", "automount", "swap",
    "desktop", "rules", "list", "sources", "d", "repo",
    "cmake", "mk", "am", "in", "ac", "m4", "spec", "nuspec",
    # バージョン管理 / パッケージ
    "gitignore", "gitattributes", "gitmodules", "dockerignore", "npmignore",
    "editorconfig", "npmrc", "yarnrc", "yarnrc.yml", "prettierrc",
    "eslintrc", "babelrc", "browserslistrc", "stylelintrc", "lock",
    "sum", "mod", "gemspec", "podspec", "cartfile", "podfile",
    # セキュリティ / 鍵(テキスト PEM 等)
    "pem", "pub", "crt", "cer", "csr", "key", "asc", "gpg", "sig", "p7b",
    # ゲームサーバー / ゲーム開発
    "mcmeta", "mcfunction", "mcfunc", "snbt", "vdf", "acf", "vmt", "res", "gi",
    "sp", "sma", "inc", "sqf", "sqm", "pwn", "nut", "gsc", "cfg", "kv",
    "fgd", "qc", "smd", "qc_", "vmf", "bsp", "wad", "pk3",
    # 科学 / 統計
    "r", "jl", "m", "mat", "sas", "do", "ado", "dta", "ipynb",
    # その他ソース
    "pas", "pp", "inc", "asm", "s", "S", "f", "f90", "f95", "f03", "for",
    "f77", "f08", "nasm", "s", "ld", "map", "lds",
    # ビルド / CI
    "yml", "yaml",  # 重複だが明示
    "jenkinsfile", "groovy",
    # LaTeX / 文書
    "sty", "cls", "ltx", "dtx", "ins",
    # メール / フィード
    "eml", "mbox", "ics", "vcf",
    # ログ / トレース
    "trace", "out", "err", "stderr", "stdout",
    # エディタ / IDE
    "sublime-project", "sublime-workspace", "code-workspace",
    "iml", "xcconfig", "pbxproj", "storyboard", "xib",
    # テンプレート
    "tmpl", "template", "tpl",
    # ローカライズ
    "po", "pot", "mo", "strings",
    # 帳票 / マークアップ系データ
    "wiki", "mediawiki", "creole",
    # Hashi / プロジェクト
    "windsurfrules",
})

# 拡張子が無い、または拡張子がファイル名そのものの既知テキスト。
TEXT_BASENAMES = frozenset({
    "makefile", "gnumakefile", "dockerfile", "containerfile",
    "gemfile", "rakefile", "vagrantfile", "procfile", "brewfile", "buildfile",
    "cmakelists.txt", "jenkinsfile", "vagrantfile", "justfile",
    "readme", "license", "licence", "changelog", "authors", "contributing",
    "copying", "notice", "credits", "todo", "bugs", "news", "history",
    "dockerfile", "containerfile",
    # ドットファイル(拡張子抽出で basename 全体になるもの)
    ".gitignore", ".gitattributes", ".gitmodules", ".dockerignore",
    ".editorconfig", ".env", ".env.example", ".env.local", ".env.development",
    ".flake8", ".pylintrc", ".coveragerc", ".npmrc", ".yarnrc",
    ".bashrc", ".zshrc", ".profile", ".bash_profile", ".bash_aliases",
    ".vimrc", ".gvimrc", ".tmux.conf", ".inputrc", ".curlrc",
    "bashrc", "profile",  # _lang_for 互換
})

# 中身次第でテキスト/バイナリになり得る拡張子(Issue #122)。
AMBIGUOUS_EXTENSIONS = frozenset({
    "dat", "db", "sav", "save", "bin", "data", "cache", "idx", "index",
    "pak", "bak", "dump", "store", "state", "meta", "nbt", "mca", "mcr",
    "region", "schematic", "schem", "booster", "dat_old",
})


def _basename(name: str) -> str:
    return os.path.basename(name).lower()


def _extension(name: str) -> str:
    """拡張子(ドット無し、小文字)。`.gitignore` は `gitignore`。"""
    base = _basename(name)
    if base.startswith(".") and base.count(".") == 1:
        return base.lstrip(".")
    if "." not in base:
        return ""
    return base.rsplit(".", 1)[-1]


def looks_text_filename(name: str) -> bool:
    """リモートファイル名から「テキストとして扱う」かを判定する。"""
    base = _basename(name)
    if base in TEXT_BASENAMES:
        return True
    ext = _extension(name)
    if ext and ext in TEXT_EXTENSIONS:
        return True
    # 拡張子なしはテキスト扱い(汎用エディタとして開けるようにする)
    if "." not in base:
        return True
    return False


def should_edit_internally(name: str) -> bool:
    """内蔵エディタ(テキスト or 16 進)で開くべきか。"""
    if looks_text_filename(name):
        return True
    return _extension(name) in AMBIGUOUS_EXTENSIONS


def language_for(path: str) -> str:
    """シンタックスハイライト用の言語 ID を返す。"""
    base = _basename(path)
    ext = _extension(path)

    if ext in ("py", "pyw", "pyi", "pyx", "pxd", "pxi"):
        return "python"
    if ext in ("rb", "erb", "gemspec", "rake", "podspec"):
        return "ruby"
    if ext in ("php", "phtml"):
        return "php"
    if ext in ("lua", "luau"):
        return "lua"
    if ext in ("sql", "psql", "mysql", "ddl", "dml"):
        return "sql"
    if ext in ("css", "scss", "sass", "less", "styl", "pcss"):
        return "css"
    if ext in ("html", "htm", "xhtml", "xml", "xsl", "xslt", "svg", "xsd",
               "wsdl", "plist"):
        return "markup"
    if ext in ("md", "markdown", "mdx", "rst", "adoc", "asciidoc", "org",
               "tex", "bib", "wiki", "mediawiki"):
        return "markup"
    if ext in ("json", "jsonc", "json5", "jsonl", "ndjson", "geojson",
               "ipynb"):
        return "json"
    if ext in ("c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "ino",
               "java", "kt", "kts", "scala", "sc", "cs", "fs", "fsx", "fsi",
               "go", "rs", "swift", "dart", "nim", "zig", "v", "vala", "d",
               "m", "mm", "pas", "pp", "asm", "s", "S", "f", "f90", "f95",
               "f03", "for", "cu", "cuh", "hs", "lhs", "ml", "mli", "elm",
               "ex", "exs", "erl", "hrl", "clj", "cljs", "cljc", "jl",
               "groovy", "gradle", "vb", "vbs", "bas"):
        return "c"
    if ext in ("js", "jsx", "mjs", "cjs", "ts", "tsx", "vue", "svelte",
               "astro", "graphql", "gql"):
        return "js"
    if ext in ("sh", "bash", "zsh", "fish", "awk", "sed", "ps1", "psm1",
               "psd1", "bat", "cmd", "r", "R", "dockerfile", "containerfile",
               "jenkinsfile", "tf", "hcl", "cmake") or base in (
            "bashrc", ".bashrc", "profile", ".profile", "dockerfile",
            "containerfile", "makefile", "gnumakefile", "jenkinsfile",
            "gemfile", "rakefile", "vagrantfile", "procfile", "brewfile",
            "justfile", "buildfile"):
        return "shell"
    if ext in ("yml", "yaml", "conf", "cfg", "ini", "toml", "env", "dotenv",
               "properties", "props", "desktop", "service", "socket", "timer",
               "target", "mount", "network", "editorconfig", "gitignore",
               "gitattributes", "gitmodules", "dockerignore", "npmrc",
               "yarnrc", "rules", "list", "tfvars", "hcl", "cnf", "config",
               "settings", "params", "opts", "inf", "reg", "manifest",
               "vdf", "acf", "mcmeta", "spec", "nuspec"):
        return "conf"
    return "plain"
