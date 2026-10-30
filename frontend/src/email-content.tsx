import { useEffect, useMemo, useRef, useState } from "react";
import type { Body } from "./types";
import { t } from "./i18n";
import { openExternal } from "./api";
import { EmailLinks } from "./email-links";

// emailLink 只解析静态网页和邮件地址链接，不解释 javascript 或其他活动协议。
export function emailLink(raw: string): string | null {
  try {
    const url = new URL(raw.startsWith("//") ? `https:${raw}` : raw);
    return ["http:", "https:", "mailto:"].includes(url.protocol)
      ? url.href
      : null;
  } catch {
    return null;
  }
}

// emailDocument 在不具备应用桥接和脚本权限的 iframe 中显示不可信正文。
export function emailDocument(
  body: Pick<Body, "html" | "text">,
  remoteImages: boolean,
  dark: boolean,
  quoted = false,
) {
  const parser = new DOMParser();
  const doc = parser.parseFromString(
    body.html ||
      `<pre>${body.text.replaceAll("&", "&amp;").replaceAll("<", "&lt;")}</pre>`,
    "text/html",
  );
  doc
    .querySelectorAll("script,iframe,object,embed,form,base,meta[http-equiv]")
    .forEach((n) => n.remove());
  doc.querySelectorAll("*").forEach((node) => {
    for (const attr of [...node.attributes]) {
      if (attr.name.startsWith("on") || attr.name === "srcdoc")
        node.removeAttribute(attr.name);
    }
    if (node instanceof HTMLAnchorElement) {
      const raw = node.getAttribute("href")?.trim() || "";
      if (raw.startsWith("#")) node.setAttribute("target", "_self");
      else {
        const link = emailLink(raw);
        if (link) node.setAttribute("href", link);
        else node.removeAttribute("href");
        node.setAttribute("target", "_blank");
      }
      node.setAttribute("rel", "noreferrer noopener");
    }
  });
  if (!remoteImages)
    doc.querySelectorAll("img").forEach((img) => {
      if (/^https?:/i.test(img.src)) {
        img.removeAttribute("src");
        img.alt = img.alt || t("远程图片已关闭");
      }
    });
  const styles = [...doc.querySelectorAll("style")]
    .map((node) => {
      const css = node.outerHTML;
      node.remove();
      return css;
    })
    .join("");
  const csp = `default-src 'none'; img-src data: ${remoteImages ? "https: http:" : ""}; style-src 'unsafe-inline'; font-src data:; base-uri 'none'; form-action 'none'`;
  return `<!doctype html><html><head><meta charset="UTF-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><style>html{color-scheme:${dark ? "dark" : "light"}}body{margin:0;padding:4px 0 20px;font:14px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color:${dark ? "#ededf1" : "#34343e"};overflow-wrap:anywhere}p{margin:0 0 10px}img{max-width:100%;height:auto}table{max-width:100%}pre{white-space:pre-wrap;font:inherit}blockquote{border-left:2px solid #d8d8e3;margin-left:0;padding-left:18px;color:#73737e}a{color:#4f46e5}${quoted ? `[data-mf-quote]>blockquote{margin:12px 0 0;padding:0;border:0;color:inherit}[data-mf-quote]>p:first-child{font-size:12px;color:${dark ? "#aaaabb" : "#73737e"}}` : ""}</style>${styles}</head><body>${doc.body.innerHTML}</body></html>`;
}
// useEmailTheme 让阅读正文与引用预览使用同一系统主题判断。
export function useEmailTheme(theme: unknown) {
  const [systemDark, setSystemDark] = useState(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  );
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const change = () => setSystemDark(media.matches);
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);
  return theme === "dark" || (theme === "system" && systemDark);
}

// EmailContent 在无脚本的隔离文档中保留 HTML，引用随正文高度伸展，滚动统一由写信区域承担。
export function EmailContent({
  html,
  text,
  title,
  remoteImages,
  dark,
  quoted = false,
  onLinkError,
}: {
  html: string | null;
  text: string;
  title: string;
  remoteImages: boolean;
  dark: boolean;
  quoted?: boolean;
  onLinkError?: (message: string) => void;
}) {
  const cleanup = useRef<(() => void) | undefined>(undefined);
  const frame = useRef<HTMLIFrameElement>(null);
  const [loaded, setLoaded] = useState(0);
  const [height, setHeight] = useState(180);
  const source = useMemo(
    () => emailDocument({ html, text }, remoteImages, dark, quoted),
    [html, text, remoteImages, dark, quoted],
  );
  useEffect(() => () => cleanup.current?.(), []);
  return (
    <div className={`email-frame ${quoted ? "quoted" : ""}`}>
      <iframe
        ref={frame}
        title={title}
        sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
        referrerPolicy="no-referrer"
        srcDoc={source}
        style={quoted ? { height } : undefined}
        onLoad={(event) => {
          cleanup.current?.();
          const doc = event.currentTarget.contentDocument;
          if (!doc?.body) return;
          setLoaded((value) => value + 1);
          // 正文没有脚本权限；由宿主监听链接点击并交给系统浏览器，不让 iframe 自行开新窗口。
          const navigate = (event: MouseEvent) => {
            const anchor = (event.target as Element | null)?.closest?.(
              "a[href]",
            );
            if (!anchor) return;
            const raw = anchor.getAttribute("href")?.trim() || "";
            if (raw.startsWith("#")) return;
            event.preventDefault();
            if (event.type === "auxclick" && event.button !== 1) return;
            const url = emailLink(raw);
            if (!url) {
              onLinkError?.(t("已阻止不安全链接"));
              return;
            }
            void openExternal(url).catch(() =>
              onLinkError?.(t("无法打开链接")),
            );
          };
          doc.addEventListener("click", navigate, true);
          doc.addEventListener("auxclick", navigate, true);
          let observer: ResizeObserver | undefined;
          const resize = () =>
            setHeight(
              Math.max(
                160,
                Math.min(
                  10000,
                  Math.ceil(doc.body.getBoundingClientRect().height) + 24,
                ),
              ),
            );
          if (quoted) {
            observer = new ResizeObserver(resize);
            observer.observe(doc.body);
            resize();
          }
          cleanup.current = () => {
            observer?.disconnect();
            doc.removeEventListener("click", navigate, true);
            doc.removeEventListener("auxclick", navigate, true);
          };
        }}
      />
      <EmailLinks frame={frame} loaded={loaded} onError={onLinkError} />
    </div>
  );
}
