import { Mail } from "lucide-react";
import type { Account } from "./types";

// accountProvider 优先按实际服务器识别企业邮箱，再按邮箱域名回退。
export function accountProvider(account: Account) {
  const hosts = [account.imap.host, account.smtp.host].map((h) =>
    h.toLowerCase().replace(/\.$/, ""),
  );
  const domain = account.email.split("@").at(-1)?.toLowerCase() || "";
  const matches = (domains: string[], values = hosts) =>
    values.some((host) =>
      domains.some((d) => host === d || host.endsWith(`.${d}`)),
    );
  const identify = (values: string[]) => {
    if (matches(["exmail.qq.com", "wecom.work"], values)) return "exmail";
    if (
      matches(
        ["qiye.163.com", "qiye163.com", "hz.163.com", "qy.163.com"],
        values,
      )
    )
      return "qiye163";
    if (matches(["qiye.aliyun.com", "mxhichina.com", "alimail.com"], values))
      return "aliqiye";
    if (matches(["qq.com", "foxmail.com"], values)) return "qq";
    if (matches(["163.com", "126.com", "yeah.net"], values)) return "netease";
    if (matches(["gmail.com", "googlemail.com"], values)) return "gmail";
    if (
      matches(
        ["outlook.com", "office365.com", "hotmail.com", "live.com", "msn.com"],
        values,
      )
    )
      return "outlook";
    if (matches(["icloud.com", "me.com", "mac.com"], values)) return "icloud";
    if (matches(["aliyun.com"], values)) return "aliyun";
    if (matches(["yahoo.com", "yahoodns.net"], values)) return "yahoo";
    if (matches(["zoho.com", "zoho.eu", "zoho.com.cn"], values)) return "zoho";
    return "other";
  };
  const provider = identify(hosts);
  return provider === "other" ? identify([domain]) : provider;
}

// ProviderIcon 以代码绘制简化厂商标记，不加载图片或第三方图标资源。
export function ProviderIcon({
  account,
  provider: selected,
}: {
  account?: Account;
  provider?: string;
}) {
  const provider = account ? accountProvider(account) : selected || "other";
  const enterprise = ["exmail", "qiye163", "aliqiye"].includes(provider);
  return (
    <span
      className={`provider-icon provider-${provider}`}
      data-provider={provider}
      aria-hidden="true"
    >
      <svg viewBox="0 0 24 24" fill="none">
        {provider === "gmail" ? (
          <>
            <path
              d="M4 18V7l8 6 8-6v11"
              stroke="#4285f4"
              strokeWidth="3"
              strokeLinejoin="round"
            />
            <path d="m4 7 8 6 8-6" stroke="#ea4335" strokeWidth="3" />
            <path d="M20 8v10" stroke="#34a853" strokeWidth="3" />
            <path d="m16 10 4-3" stroke="#fbbc04" strokeWidth="3" />
          </>
        ) : provider === "qq" ? (
          <>
            <ellipse cx="12" cy="12" rx="6" ry="8" fill="#252733" />
            <ellipse cx="12" cy="14" rx="4" ry="5" fill="white" />
            <circle cx="10" cy="7.5" r="1.2" fill="white" />
            <circle cx="14" cy="7.5" r="1.2" fill="white" />
            <path d="m9 10 3 1 3-1" stroke="#ffc447" strokeWidth="2" />
            <path d="m6 12 6 2 6-2" stroke="#ed4949" strokeWidth="2" />
          </>
        ) : provider === "exmail" ? (
          <path
            d="M9 6H4v12h5m6-12h5v12h-5M9 6l6 12M9 18l6-12"
            stroke="#2d7cf6"
            strokeWidth="2.2"
            strokeLinejoin="round"
          />
        ) : ["aliqiye", "aliyun"].includes(provider) ? (
          <path
            d="M9 6H4v12h5m6-12h5v12h-5M9 12h6"
            stroke="#ff6a00"
            strokeWidth="2.4"
          />
        ) : ["netease", "qiye163"].includes(provider) ? (
          <>
            <rect x="3" y="5" width="18" height="14" rx="3" fill="#d8282f" />
            <path d="m5 8 7 5 7-5" stroke="white" strokeWidth="1.6" />
          </>
        ) : provider === "outlook" ? (
          <>
            <path d="M10 4h11v16H10z" fill="#28a8ea" />
            <path d="m10 9 6 5 5-5" stroke="white" strokeWidth="1.4" />
            <path d="M2 6h12v13H2z" fill="#0078d4" />
            <ellipse
              cx="8"
              cy="12.5"
              rx="2.5"
              ry="3.5"
              stroke="white"
              strokeWidth="1.5"
            />
          </>
        ) : provider === "icloud" ? (
          <path
            d="M6 18a4 4 0 0 1-.2-8A6.5 6.5 0 0 1 18 8.5 4.8 4.8 0 0 1 18 18Z"
            fill="#3693f3"
          />
        ) : provider === "yahoo" ? (
          <path
            d="m4 5 5 8v6h3v-6l5-8m3 9v2m0 3v1"
            stroke="#720e9e"
            strokeWidth="2.5"
          />
        ) : provider === "zoho" ? (
          <>
            <rect
              x="3"
              y="7"
              width="8"
              height="11"
              rx="1"
              stroke="#e42527"
              strokeWidth="2"
            />
            <rect
              x="9"
              y="5"
              width="8"
              height="11"
              rx="1"
              stroke="#089949"
              strokeWidth="2"
            />
            <path d="M16 8h5v11h-8v-3" stroke="#226db4" strokeWidth="2" />
          </>
        ) : (
          <Mail
            x="3"
            y="3"
            width="18"
            height="18"
            stroke="currentColor"
            strokeWidth="1.7"
          />
        )}
        {enterprise && (
          <circle
            cx="20"
            cy="20"
            r="3"
            fill={
              provider === "aliqiye"
                ? "#ff6a00"
                : provider === "exmail"
                  ? "#2d7cf6"
                  : "#d8282f"
            }
            stroke="white"
          />
        )}
      </svg>
    </span>
  );
}
