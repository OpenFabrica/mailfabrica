import { t } from "./i18n";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import {
  X,
  Mail,
  Inbox,
  FilePen,
  Send,
  Trash2,
  ShieldAlert,
  Archive,
  Folder as FolderIcon,
} from "lucide-react";
// IconButton 为无文字工具提供一致的可访问名称和提示。
export function IconButton({
  label,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  label: string;
  children: ReactNode;
}) {
  return (
    <button
      {...props}
      className={`icon-button ${props.className || ""}`}
      title={label}
      aria-label={label}
    >
      {children}
    </button>
  );
}
// Dialog 使用模态语义并允许 Escape 关闭，业务提交由外层处理。
export function Dialog({
  title,
  children,
  onClose,
  wide = false,
  className = "",
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
  className?: string;
}) {
  return (
    <div
      className="scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={`dialog ${wide ? "wide" : ""} ${className}`}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onClose();
          }
        }}
      >
        <header>
          <h2>{title}</h2>
          <IconButton label={t("关闭")} onClick={onClose}>
            <X />
          </IconButton>
        </header>
        {children}
      </section>
    </div>
  );
}
// folderIcon 统一文件夹语义与原创代码图标的调用位置。
export function folderIcon(role: string) {
  const Icon =
    (
      {
        inbox: Inbox,
        drafts: FilePen,
        sent: Send,
        trash: Trash2,
        junk: ShieldAlert,
        archive: Archive,
      } as Record<string, typeof Mail>
    )[role] || FolderIcon;
  return <Icon size={16} />;
}
// folderName 使用服务器角色翻译，不改写用户自建目录名。
export function folderName(path: string, role: string, english = false) {
  const names: Record<string, string> = english
    ? {
        inbox: "Inbox",
        drafts: "Drafts",
        sent: "Sent",
        trash: "Trash",
        junk: "Junk",
        archive: "Archive",
      }
    : {
        inbox: t("收件箱"),
        drafts: t("草稿箱"),
        sent: t("已发送"),
        trash: t("已删除"),
        junk: t("垃圾邮件"),
        archive: t("归档"),
      };
  return names[role] || path.split("/").at(-1) || path;
}
// dateLabel 按本地日期显示列表时间。
export function dateLabel(date: number, english = false) {
  const time = new Date(date * 1000);
  const now = new Date();
  return time.toDateString() === now.toDateString()
    ? time.toLocaleTimeString(english ? "en" : "zh-CN", {
        hour: "2-digit",
        minute: "2-digit",
      })
    : time.toLocaleDateString(english ? "en" : "zh-CN", {
        month: "short",
        day: "numeric",
      });
}
