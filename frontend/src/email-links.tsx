import { useEffect, useState, type RefObject } from "react";
import { nativeBridge, openExternal } from "./api";
import { t } from "./i18n";

// LinkRect 只保存可见静态链接的点击区域，真实 URL 在点击时重新从正文命中。
interface LinkRect {
  key: string;
  left: number;
  top: number;
  width: number;
  height: number;
  href: string;
  text: string;
}

// EmailLinks 处理 WebKit 禁脚本时拒绝正文 DOM 回调的情况；点击层位于可信父页面，不放开邮件脚本。
export function EmailLinks({
  frame,
  loaded,
  onError,
}: {
  frame: RefObject<HTMLIFrameElement | null>;
  loaded: number;
  onError?: (message: string) => void;
}) {
  const [links, setLinks] = useState<LinkRect[]>([]);
  useEffect(() => {
    if (!nativeBridge || !loaded) return;
    const element = frame.current,
      doc = element?.contentDocument;
    if (!element || !doc?.body) return;
    const anchors = [...doc.querySelectorAll<HTMLAnchorElement>("a[href]")];
    anchors.forEach((anchor) => {
      anchor.tabIndex = -1;
    });
    let animation = 0,
      previous = "",
      changed = true;
    const observer = new ResizeObserver(() => {
      changed = true;
    });
    observer.observe(doc.body);
    observer.observe(element);
    // 父页面能读取隔离文档的几何信息；读取滚动偏移不依赖正文内事件或脚本。
    const position = () => {
      if (document.hidden) {
        animation = requestAnimationFrame(position);
        return;
      }
      const signature = `${doc.documentElement.scrollTop}:${doc.documentElement.scrollLeft}:${doc.body.scrollTop}:${doc.body.scrollHeight}:${element.clientWidth}:${element.clientHeight}`;
      if (changed || signature !== previous) {
        changed = false;
        previous = signature;
        const css = getComputedStyle(element),
          paddingX = parseFloat(css.paddingLeft) || 0,
          paddingY = parseFloat(css.paddingTop) || 0;
        const width =
          element.clientWidth - paddingX - (parseFloat(css.paddingRight) || 0);
        const height =
          element.clientHeight -
          paddingY -
          (parseFloat(css.paddingBottom) || 0);
        const next: LinkRect[] = [];
        anchors.forEach((anchor, index) => {
          [...anchor.getClientRects()].forEach((rect, part) => {
            const left = Math.max(0, rect.left),
              top = Math.max(0, rect.top),
              right = Math.min(width, rect.right),
              bottom = Math.min(height, rect.bottom);
            if (
              right <= left ||
              bottom <= top ||
              getComputedStyle(anchor).visibility === "hidden"
            )
              return;
            next.push({
              key: `${index}-${part}`,
              left: left + paddingX,
              top: top + paddingY,
              width: right - left,
              height: bottom - top,
              href: anchor.getAttribute("href") || "",
              text:
                anchor.textContent?.trim() ||
                anchor.querySelector("img")?.alt ||
                anchor.getAttribute("href") ||
                "",
            });
          });
        });
        setLinks(next);
      }
      animation = requestAnimationFrame(position);
    };
    position();
    return () => {
      cancelAnimationFrame(animation);
      observer.disconnect();
    };
  }, [frame, loaded]);
  if (!nativeBridge) return null;
  return (
    <div className="email-link-layer">
      {links.map((link) => (
        <a
          key={link.key}
          data-email-link="true"
          href={link.href}
          aria-label={link.text}
          title={link.href}
          rel="noreferrer noopener"
          className="email-link-target"
          style={{
            left: link.left,
            top: link.top,
            width: link.width,
            height: link.height,
          }}
          onClick={(event) => {
            event.preventDefault();
            const element = frame.current,
              doc = element?.contentDocument;
            if (!element || !doc) return;
            const bounds = element.getBoundingClientRect(),
              css = getComputedStyle(element);
            // 真实指针再次命中正文，避免滚动与排版变化后打开旧位置的链接；键盘使用已聚焦链接。
            const anchor =
              event.detail === 0
                ? null
                : doc
                    .elementFromPoint(
                      event.clientX -
                        bounds.left -
                        (parseFloat(css.paddingLeft) || 0),
                      event.clientY -
                        bounds.top -
                        (parseFloat(css.paddingTop) || 0),
                    )
                    ?.closest("a[href]");
            const href =
              event.detail === 0 ? link.href : anchor?.getAttribute("href");
            if (!href) return;
            if (href.startsWith("#")) {
              try {
                doc
                  .getElementById(decodeURIComponent(href.slice(1)))
                  ?.scrollIntoView({ block: "start" });
              } catch {
                /* 无效页内锚点不导航。 */
              }
              return;
            }
            void openExternal(href).catch(() => onError?.(t("无法打开链接")));
          }}
        />
      ))}
    </div>
  );
}
