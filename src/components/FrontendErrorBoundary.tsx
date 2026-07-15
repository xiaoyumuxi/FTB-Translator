import React from "react";
import { CircleAlert } from "lucide-react";
import { frontendLog } from "../services/tauri";

export class FrontendErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    void frontendLog("error", "react_render_failed", "React 界面渲染失败", {
      error: error.message,
      component_stack: info.componentStack || "",
    });
  }

  render() {
    return this.state.failed ? (
      <main className="frontend-fatal">
        <CircleAlert />
        <h1>界面出现错误</h1>
        <p>错误已经写入 frontend.log，请重启应用后重试。</p>
      </main>
    ) : (
      this.props.children
    );
  }
}
