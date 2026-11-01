import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { LoaderCircle, RefreshCw, ShieldAlert, X } from "lucide-react";
import { IconButton } from "./components";
import type { SessionUI } from "./use-session";
// JunkDialog 以右下角非模态面板识别疑似垃圾邮件，不遮挡其它操作；用户勾选确认后才移动。
export function JunkDialog({
  ui,
  onClose,
}: {
  ui: SessionUI;
  onClose: () => void;
}) {
  const scan = ui.state?.aiJunk;
  const candidates = scan?.candidates ?? [];
  const [selected, setSelected] = useState<string[]>([]);
  const started = useRef(false);
  // 打开时若没有结果则立即识别；已有结果（含进行中）直接展示
  useEffect(() => {
    if (!started.current && !scan) void ui.run("ai.scanJunk", {});
    started.current = true;
  }, []);
  // 边识别边出结果：新候选默认勾选，已取消勾选的保持不变
  const known = useRef<string[]>([]);
  const candidateKeys = candidates.map((c) => c.key).join("\n");
  useEffect(() => {
    const keys = candidates.map((c) => c.key);
    const added = keys.filter((k) => !known.current.includes(k));
    known.current = keys;
    setSelected((v) => [...v.filter((k) => keys.includes(k)), ...added]);
  }, [candidateKeys]);
  // 关闭时清掉结果，下次打开按当时的列表重新识别
  const close = () => {
    if (scan && !scan.busy) void ui.run("ai.clearJunk", {});
    onClose();
  };
  return (
    <section
      className="junk-panel"
      role="dialog"
      aria-modal="false"
      aria-label={t("AI 清理垃圾邮件")}
      onKeyDown={(e) => {
        if (e.key === "Escape") close();
      }}
    >
      <header>
        <h2>{t("AI 清理垃圾邮件")}</h2>
        <IconButton label={t("关闭")} onClick={close}>
          <X />
        </IconButton>
      </header>
      <div className="junk-body">
        {scan?.busy && (
          <div role="status" className="junk-progress">
            <p className="junk-status">
              <LoaderCircle size={14} className="spin" />
              {t("正在识别")} {scan.done} / {scan.scanned}
              {candidates.length > 0 &&
                ` · ${t("发现")} ${candidates.length} ${t("封垃圾邮件")}`}
            </p>
            <progress value={scan.done} max={Math.max(1, scan.scanned)} />
            {scan.current && <p className="junk-current">{scan.current}</p>}
          </div>
        )}
        {scan?.error && <p className="inline-error">{scan.error}</p>}
        {scan && !scan.busy && !scan.error && candidates.length === 0 && (
          <p className="junk-status">
            {t("已检查")} {scan.scanned} {t("封邮件，未发现垃圾邮件")}
          </p>
        )}
        {candidates.length > 0 && (
          <div className="invoice-table junk-table">
            <label className="invoice-row junk-row header">
              <input
                type="checkbox"
                checked={selected.length === candidates.length}
                onChange={(e) =>
                  setSelected(
                    e.target.checked ? candidates.map((c) => c.key) : [],
                  )
                }
              />
              <span>{t("发件人")}</span>
              <span>{t("主题")}</span>
            </label>
            {candidates.map((c) => (
              <label
                className="invoice-row junk-row"
                key={c.key}
                title={`${c.from.name} <${c.from.email}>\n${c.subject}\n${c.reason}`}
              >
                <input
                  type="checkbox"
                  checked={selected.includes(c.key)}
                  onChange={(e) =>
                    setSelected((v) =>
                      e.target.checked
                        ? [...v, c.key]
                        : v.filter((k) => k !== c.key),
                    )
                  }
                />
                <span>{c.from.name || c.from.email}</span>
                <span>{c.subject}</span>
                <span className="muted junk-reason">{c.reason}</span>
              </label>
            ))}
          </div>
        )}
        <footer>
          <button
            disabled={!!scan?.busy}
            onClick={() => void ui.run("ai.scanJunk", {})}
          >
            <RefreshCw size={14} />
            {t("重新识别")}
          </button>
          <button
            className="primary"
            data-testid="junk.clean"
            disabled={!!scan?.busy || selected.length === 0}
            onClick={async () => {
              // 移动交给后台执行，提交成功后直接关闭弹窗
              const result = await ui.run("ai.cleanJunk", { keys: selected });
              if (result !== undefined) close();
            }}
          >
            <ShieldAlert size={14} />
            {t("移到垃圾邮件")} ({selected.length})
          </button>
        </footer>
      </div>
    </section>
  );
}
