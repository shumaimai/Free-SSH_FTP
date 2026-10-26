// 画面のアイコンの単一ソース。@fluentui/react-icons はこのファイルからだけ読み込む(scripts/check-icons.mjs が止める)。
// ISO/IEC 11581-1(JIS X 9303-1 の対応国際規格)の一般原則に沿って、次を守る。
//  - 1 つの機能に 1 つのアイコン。違う機能に同じアイコンを使わない。
//  - 塗り(Filled)は「選択中・有効」の状態だけに使う。通常は線(Regular)。
//  - 大きさは 20px にそろえる(閉じる×などの付属の印だけ小さくする)。
//  - アイコンだけのボタンには、名前(Tooltip)を必ず付ける。状態は色だけで示さず形も変える。
import {
  Add20Regular,
  ArrowDownload20Regular,
  ArrowImport20Regular,
  ArrowRouting20Regular,
  ArrowSync20Regular,
  ArrowUp20Regular,
  ArrowUpload20Regular,
  Attach20Regular,
  BrainCircuit20Regular,
  ChatAdd20Regular,
  Checkmark16Filled,
  Checkmark20Filled,
  CheckmarkCircle12Filled,
  Clock12Regular,
  Code20Regular,
  Delete20Regular,
  Dismiss12Regular,
  Dismiss16Regular,
  Document20Regular,
  DocumentEdit20Regular,
  Edit20Regular,
  ErrorCircle12Filled,
  Folder20Regular,
  FolderArrowUp20Regular,
  FolderOpen20Regular,
  HardDrive20Regular,
  Home20Regular,
  Key20Regular,
  Options20Regular,
  PanelLeft20Filled,
  PanelLeft20Regular,
  PanelRight20Filled,
  PanelRight20Regular,
  PlugConnected20Regular,
  PuzzlePiece20Regular,
  Save20Regular,
  SaveEdit20Regular,
  Send20Regular,
  Server20Regular,
  Settings20Regular,
  ShieldKeyhole20Regular,
  Star20Regular,
  Stop20Regular,
  SubtractCircle12Regular,
  WindowConsole20Regular,
  Wrench20Regular,
  bundleIcon,
} from '@fluentui/react-icons';

/** 機能 → アイコン。使う側は機能の名前で選ぶ。 */
export const Icon = {
  // 場所・画面
  home: Home20Regular,
  sshSession: PlugConnected20Regular, // SSH の接続(タブ)
  localTerminal: WindowConsole20Regular, // この PC の端末(タブ・ホームのボタン)
  editFile: DocumentEdit20Regular, // ファイルを編集画面で開く(タブ・ファイル一覧の「編集」・この PC のファイルを開く)
  aiModels: BrainCircuit20Regular, // AI モデルの管理
  externalTools: PuzzlePiece20Regular, // 外部連携(MCP)
  portForward: ArrowRouting20Regular, // ポート転送
  jumpHost: Server20Regular, // 踏み台(ProxyJump)
  tools: Wrench20Regular,
  settings: Settings20Regular, // 外観などの設定
  sshKey: Key20Regular,
  // 一般の操作
  add: Add20Regular,
  edit: Edit20Regular, // 登録内容(接続先・スニペット)を直す
  remove: Delete20Regular,
  close: Dismiss12Regular, // タブ・添付を閉じる(付属の小さな印)
  closeMessage: Dismiss16Regular, // お知らせ欄を閉じる
  importOld: ArrowImport20Regular,
  save: Save20Regular,
  saveAs: SaveEdit20Regular,
  // ファイル
  parentFolder: ArrowUp20Regular,
  refresh: ArrowSync20Regular,
  bookmark: Star20Regular,
  download: ArrowDownload20Regular,
  upload: ArrowUpload20Regular,
  uploadFolder: FolderArrowUp20Regular,
  pickFolder: FolderOpen20Regular,
  drive: HardDrive20Regular,
  folder: Folder20Regular,
  file: Document20Regular,
  permission: ShieldKeyhole20Regular,
  transferOptions: Options20Regular,
  // AI
  newChat: ChatAdd20Regular,
  attach: Attach20Regular,
  send: Send20Regular,
  stop: Stop20Regular,
  snippets: Code20Regular,
  // 切り替え(押されている間は塗りの形になる)
  dockPanel: bundleIcon(PanelRight20Filled, PanelRight20Regular),
  twoPane: bundleIcon(PanelLeft20Filled, PanelLeft20Regular),
  // 選択中の印
  selected: Checkmark20Filled,
  selectedSmall: Checkmark16Filled,
  // 接続の状態(色だけでなく形でも区別する)
  stateConnected: CheckmarkCircle12Filled,
  stateConnecting: Clock12Regular,
  stateError: ErrorCircle12Filled,
  stateClosed: SubtractCircle12Regular,
} as const;
