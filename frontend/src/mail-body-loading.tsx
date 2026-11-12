import { LoaderCircle } from "lucide-react";
import { t } from "./i18n";

// MailBodyLoading 覆盖打开请求、正文下载及文档排版阶段，缓存命令也有可见的阅读过渡。
export function MailBodyLoading() {
  return (
    <div
      className="mail-body-loading"
      data-testid="reader.loading"
      role="status"
      aria-live="polite"
    >
      <LoaderCircle className="spin" size={24} />
      <span>{t("正在加载邮件正文…")}</span>
      <div className="mail-body-skeleton" aria-hidden="true">
        <i />
        <i />
        <i />
      </div>
    </div>
  );
}
