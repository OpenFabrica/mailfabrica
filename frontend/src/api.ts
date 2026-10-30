import type { Params, Snapshot } from "./types";
const service = "github.com/openfabrica/mailfabrica/internal/desktop.Service";
// nativeBridge 只在宿主已注入的环境中加载 Wails runtime。
export const nativeBridge = Boolean(
  (window as unknown as { _wails?: { flags?: unknown } })._wails?.flags,
);
// invoke 让浏览器验收与桌面共用相同 Go 会话，禁止前端演示数据替代后端。
async function invoke<T>(method: string, args: unknown[] = []): Promise<T> {
  if (nativeBridge) {
    const { Call } = await import("@wailsio/runtime");
    const text = (await Call.ByName(`${service}.${method}`, ...args)) as string;
    return JSON.parse(text) as T;
  }
  const response = await fetch(`/api/${method}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args),
  });
  const result = (await response.json()) as { result: T; error?: string };
  if (!response.ok || result.error)
    throw new Error(result.error || `HTTP ${response.status}`);
  return result.result;
}
// execute 是所有用户业务操作的唯一入口。
export const execute = <T = unknown>(command: string, params: Params = {}) =>
  invoke<T>("Execute", [command, JSON.stringify(params)]);
// snapshot 获取不可变的后端状态快照。
export const snapshot = () => invoke<Snapshot>("Snapshot");
// pickFile 由宿主原生对话框选择附件、导出目录或保存位置。
export const pickFile = (mode: "open" | "save" | "directory", name = "") =>
  invoke<string>("PickFile", [mode, name]);
// openExternal 仅由用户点击触发，后端再次校验外部链接协议。
export async function openExternal(raw: string) {
  const url = new URL(raw);
  if (!["http:", "https:", "mailto:"].includes(url.protocol))
    throw new Error("链接协议不支持");
  if (!nativeBridge) {
    window.open(url.href, "_blank", "noopener,noreferrer");
    return null;
  }
  return invoke<null>("OpenExternal", [url.href]);
}
