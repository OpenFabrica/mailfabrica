import { t } from "./i18n";
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app";
import "./style.css";
// render 将应用挂载到自有根节点，不注入邮件正文脚本。
const root = document.getElementById("root");
if (root)
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <React.Suspense
        fallback={<main className="loading-screen">{t("正在打开…")}</main>}
      >
        <App />
      </React.Suspense>
    </React.StrictMode>,
  );
