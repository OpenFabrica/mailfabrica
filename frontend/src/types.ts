// 与 Go 快照对应的交付类型，业务状态由命令引擎维护。
export type Params = Record<string, unknown>;
export interface Address {
  name: string;
  email: string;
}
export interface Key {
  account: string;
  folder: string;
  uid: number;
}
export interface Flags {
  seen: boolean;
  flagged: boolean;
  answered: boolean;
  draft: boolean;
  deleted: boolean;
}
export interface Message {
  key: Key;
  messageId: string;
  subject: string;
  from: Address;
  to: Address[];
  date: number;
  flags: Flags;
  preview: string;
  hasAttachments: boolean;
}
export interface Attachment {
  index: number;
  name: string;
  mime: string;
  size: number;
  inline: boolean;
  contentId: string | null;
}
export interface Body {
  subject: string;
  from: Address[];
  to: Address[];
  cc: Address[];
  text: string;
  html: string | null;
  attachments: Attachment[];
}
export interface Row {
  group: string | null;
  count: number;
  unread: number;
  flagged: boolean;
  keys: string[];
  message: Message;
  senders: string[];
}
export interface Server {
  host: string;
  port: number;
  security: string;
}
export interface Account {
  id: string;
  email: string;
  name: string;
  login: string;
  signature: string;
  imap: Server;
  smtp: Server;
}
export interface Folder {
  account: string;
  path: string;
  role: string;
  unread: number;
  total: number;
  selectable: boolean;
}
export interface Compose {
  id: number;
  account: string;
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
  html: string;
  rich: boolean;
  quote: { html: string; text: string } | null;
  attachments: Attachment[];
  sending: boolean;
  error: string | null;
  dirty: boolean;
  aiBusy: boolean;
  aiSuggestion: string | null;
  aiError: string | null;
}
export interface UIRequest {
  kind: string;
  id?: number;
  name?: string;
  params?: unknown;
}
export interface Snapshot {
  revision: number;
  demo: boolean;
  accounts: Account[];
  folders: Record<string, Folder[]>;
  view: {
    scope: { kind: string; account?: string; folder?: string };
    query: string;
    messages: Message[];
    rows: Row[];
    selected: string[];
    focused: string | null;
    group: string | null;
  };
  // 跨账户全局计数，不随当前列表范围变化
  counts: { unread: number; flagged: number };
  // 已显式加载的系统日历数据；events 为 null 表示尚未加载
  calendar: {
    supported: boolean;
    since: string;
    until: string;
    events: CalendarEvent[] | null;
  };
  message: { meta: Message; body?: Body; state: string; error?: string } | null;
  composes: Compose[];
  prefs: Record<string, string | number | boolean | string[]>;
  status: {
    busy: boolean;
    text: string;
    errors: [string, string][];
    lastSync: number;
    busyAccounts: string[];
  };
  requests: UIRequest[];
  aiGroupSummary: {
    group: string;
    keys: string[];
    busy: boolean;
    text: string | null;
    error: string | null;
  } | null;
  aiReply: {
    key: string;
    seq: number;
    busy: boolean;
    text: string | null;
    error: string | null;
  } | null;
  invoice: {
    busy: boolean;
    total: number;
    done: number;
    files: number;
    failed: number;
    csv: string | null;
    error: string | null;
  };
  update: {
    phase: string;
    release: { version: string; notes: string } | null;
    downloaded: number;
    installer: string | null;
    error: string | null;
  };
}
export interface CalendarEvent {
  id: string;
  title: string;
  calendar: string;
  start: number;
  end: number;
  allDay: boolean;
  location: string;
}
export interface CalendarDay {
  date: string;
  lunar: string;
  events: CalendarEvent[];
  messages: {
    key: string;
    account: string;
    subject: string;
    from: Address;
    date: number;
    unread: boolean;
    flagged: boolean;
  }[];
  messageCount: number;
}
// keyString 保留文件夹可能含斜线的稳定消息键。
export const keyString = (k: Key) => `${k.account}/${k.uid}/${k.folder}`;
