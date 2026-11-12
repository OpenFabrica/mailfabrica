import { t } from "./i18n";
import type { SessionUI } from "./use-session";

// DeletionStatus 统一底栏进度、结果和错误，每次只渲染一条通知，避免旧错误覆盖新状态。
export function DeletionStatus({ ui }: { ui: SessionUI }) {
  const messages = ui.messageDeletions.length > 0 || !!ui.messageDeleteFeedback;
  const pending =
    ui.messageDeletions.length > 0 || ui.folderDeletions.length > 0;
  const latestError = ui.state?.status.errors.at(-1);
  const currentFailure =
    pending &&
    latestError &&
    (ui.messageDeletions.some(
      (job) =>
        job.task === latestError[0] ||
        (job.accounts.includes(latestError[0]) &&
          (ui.state?.status.errorRevision || 0) > (job.revision || 0)),
    ) ||
      ui.folderDeletions.some(
        (job) =>
          (ui.state?.status.errorRevision || 0) > job.errorRevision &&
          (latestError[0] === `folder.mutate:${job.account}` ||
            latestError[0] === job.account),
      ))
      ? latestError[1]
      : null;
  const failure =
    currentFailure ||
    ui.messageDeleteFeedback?.error ||
    ui.deleteFeedback?.error;
  const en = ui.state?.prefs.language === "en";
  let text: string | undefined;
  if (pending) {
    text = currentFailure
      ? t("删除连接异常，正在核对服务器结果…")
      : ui.deletingFolders.size === 0 && ui.changingFolders.length === 0
        ? t("正在核对删除结果…")
        : ui.messageDeletions.length
          ? en
            ? `Deleting ${ui.deletingKeys.size} messages in background…`
            : `正在后台删除 ${ui.deletingKeys.size} 封邮件…`
          : t("正在后台删除邮件…");
  } else if (failure) {
    text = `${t("删除未全部完成，请检查剩余邮件")}: ${failure}`;
  } else if (messages) {
    text = en
      ? `Deleted ${ui.messageDeleteFeedback?.keys.length} messages`
      : `已删除 ${ui.messageDeleteFeedback?.keys.length} 封邮件`;
  } else if (ui.deleteFeedback) {
    text = t("删除完成");
  } else if (!ui.error) {
    text = latestError?.[1];
  }
  if (!text) return null;
  return (
    <footer
      className={`mail-operation-status ${failure || (!pending && !messages && !ui.deleteFeedback) ? "error" : ""}`}
      data-testid={
        messages
          ? "message.deleteStatus"
          : ui.folderDeletions.length || ui.deleteFeedback
            ? "folder.deleteStatus"
            : "session.status"
      }
      role="status"
      aria-live="polite"
      title={failure ? `${text}\n${failure}` : text}
    >
      <span>{text}</span>
    </footer>
  );
}
