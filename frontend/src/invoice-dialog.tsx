import { t } from "./i18n";
import { useEffect, useState } from "react";
import { Download, FolderOpen } from "lucide-react";
import { Dialog } from "./components";
import { execute, pickFile } from "./api";
import { keyString, type Message } from "./types";
import type { SessionUI } from "./use-session";
// InvoiceDialog 先选择日期和邮件，再导出实际附件与 CSV。
export function InvoiceDialog({
  ui,
  onClose,
}: {
  ui: SessionUI;
  onClose: () => void;
}) {
  const [since, setSince] = useState("");
  const [until, setUntil] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [dir, setDir] = useState("");
  const status = ui.state?.invoice;
  useEffect(() => {
    void execute<{
      messages: Message[];
    }>("invoice.list", { since, until })
      .then((result) => {
        setMessages(result.messages);
        setSelected(result.messages.map((m) => keyString(m.key)));
      })
      .catch((e) => ui.setError(String(e)));
  }, [since, until]);
  return (
    <Dialog title={t("导出发票")} onClose={onClose} wide>
      <div className="invoice-content">
        <div className="date-range">
          <label>
            {t("开始日期")}
            <input
              type="date"
              value={since}
              onChange={(e) => setSince(e.target.value)}
            />
          </label>
          <span>{t("至")}</span>
          <label>
            {t("结束日期")}
            <input
              type="date"
              value={until}
              onChange={(e) => setUntil(e.target.value)}
            />
          </label>
          <button
            onClick={() => {
              const now = new Date();
              setSince(
                `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-01`,
              );
              const last = new Date(now.getFullYear(), now.getMonth() + 1, 0);
              setUntil(
                `${last.getFullYear()}-${String(last.getMonth() + 1).padStart(2, "0")}-${last.getDate()}`,
              );
            }}
          >
            {t("本月")}
          </button>
        </div>
        <div className="invoice-table">
          <label className="invoice-row header">
            <input
              type="checkbox"
              checked={
                messages.length > 0 && selected.length === messages.length
              }
              onChange={(e) =>
                setSelected(
                  e.target.checked ? messages.map((m) => keyString(m.key)) : [],
                )
              }
            />
            <span>{t("发件人")}</span>
            <span>{t("主题")}</span>
            <span>{t("日期")}</span>
          </label>
          {messages.map((m) => (
            <label className="invoice-row" key={keyString(m.key)}>
              <input
                type="checkbox"
                checked={selected.includes(keyString(m.key))}
                onChange={(e) =>
                  setSelected((v) =>
                    e.target.checked
                      ? [...v, keyString(m.key)]
                      : v.filter((k) => k !== keyString(m.key)),
                  )
                }
              />
              <span>{m.from.name || m.from.email}</span>
              <span>{m.subject}</span>
              <time>{new Date(m.date * 1000).toLocaleDateString()}</time>
            </label>
          ))}
          {messages.length === 0 && (
            <p className="empty">{t("此日期范围内没有发票邮件")}</p>
          )}
        </div>
        <label className="export-directory">
          {t("保存到")}
          <input
            placeholder={t("导出目录的绝对路径")}
            value={dir}
            onChange={(e) => setDir(e.target.value)}
          />
          <button
            onClick={() => {
              void pickFile("directory")
                .then(setDir)
                .catch((e) => ui.setError(String(e)));
            }}
          >
            <FolderOpen size={15} />
            {t("选择")}
          </button>
        </label>
        {status?.busy && (
          <div className="export-progress">
            <progress value={status.done} max={Math.max(1, status.total)} />
            <span>
              {status.done} / {status.total}
            </span>
          </div>
        )}
        {status?.csv && (
          <p>
            {t("已导出")}
            {status.files}
            {t("个文件 ·")}
            {status.csv}
          </p>
        )}
        {status?.error && <p className="inline-error">{status.error}</p>}
        <footer>
          <button onClick={onClose}>{t("关闭")}</button>
          <button
            className="primary"
            disabled={!dir || selected.length === 0 || status?.busy}
            onClick={() => {
              void ui.run("invoice.export", {
                dir,
                keys: selected,
                since,
                until,
              });
            }}
          >
            <Download size={14} />
            {t("导出")}
            {selected.length}
            {t("封")}
          </button>
        </footer>
      </div>
    </Dialog>
  );
}
