import { useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  BookOpen,
  Eye,
  EyeOff,
  FileSearch,
  FolderOpen,
  KeyRound,
  Languages,
  RefreshCw,
  Save,
  Settings,
  ShieldCheck,
  Sparkles,
} from "lucide-react";
import {
  providerOptions,
  type LogLevel,
  type Provider,
  type SettingsData,
} from "../models/settings";
import { call, frontendLog } from "../services/tauri";

export function SettingsPage({
  value,
  onChange,
  onProviderChange,
  onSave,
  notify,
}: {
  value: SettingsData;
  onChange: (value: SettingsData) => void;
  onProviderChange: (value: Provider) => void;
  onSave: () => void;
  notify: (value: string) => void;
}) {
  const [show, setShow] = useState(false);
  const [credentialStatus, setCredentialStatus] = useState("");
  const [logDirectory, setLogDirectory] = useState("正在读取应用目录…");
  const update = (key: keyof SettingsData, nextValue: string) => onChange({ ...value, [key]: nextValue });
  const preset = providerOptions[value.provider];
  const needsCredential = !!preset.credentialLabel;

  useEffect(() => {
    call<{ directory: string; backend: string; frontend: string }>("logs-info")
      .then((result) => setLogDirectory(`${result.directory} · ${result.backend} / ${result.frontend}`))
      .catch((error) => setLogDirectory(String(error)));
  }, []);

  async function toggleCredential() {
    if (show) {
      setShow(false);
      return;
    }
    if (value.api_key || value.api_key_changed) {
      setShow(true);
      return;
    }
    setCredentialStatus("正在读取钥匙串…");
    try {
      const saved = await call<{ api_key: string; has_api_key: boolean }>("provider-credential", {
        provider: value.provider,
      });
      onChange({
        ...value,
        api_key: saved.api_key,
        api_key_changed: false,
        has_api_key: saved.has_api_key,
      });
      setShow(true);
      setCredentialStatus(saved.has_api_key ? "已加载到本次应用会话" : "钥匙串中没有当前服务的 Key");
      void frontendLog("info", "credential_viewed", "用户查看了当前服务的凭证状态", {
        provider: value.provider,
        has_api_key: saved.has_api_key,
      });
    } catch (error) {
      setCredentialStatus(String(error));
    }
  }

  function changeApiKey(api_key: string) {
    onChange({ ...value, api_key, api_key_changed: true, has_api_key: !!api_key.trim() });
    setCredentialStatus(api_key.trim() ? "新 Key 待保存" : "保存后将删除当前服务的 Key");
  }

  async function chooseGlossary() {
    const path = await open({
      multiple: false,
      directory: false,
      title: "选择 Minecraft 词表 JSON",
      filters: [{ name: "JSON 词表", extensions: ["json"] }],
    });
    if (typeof path === "string") {
      onChange({ ...value, glossary_path: path });
      void frontendLog("info", "glossary_selected", "用户选择了词表文件", { path });
    }
  }

  async function resetGlossary() {
    const result = await call<{ path: string }>("default-glossary");
    onChange({ ...value, glossary_path: result.path });
    void frontendLog("info", "glossary_reset", "用户恢复了默认词表路径", { path: result.path });
  }

  async function openLogs() {
    try {
      await call("logs-open");
      void frontendLog("info", "logs_opened", "用户打开了日志目录");
      notify("已打开日志目录");
    } catch (error) {
      notify(String(error));
    }
  }

  async function exportLogs() {
    const target = await save({
      title: "导出诊断日志",
      defaultPath: "ftb-translater-logs.zip",
      filters: [{ name: "ZIP 压缩包", extensions: ["zip"] }],
    });
    if (!target) return;
    try {
      await call("logs-export", { path: target });
      void frontendLog("info", "logs_exported", "用户导出了前后端诊断日志", { path: target });
      notify("诊断日志已导出");
    } catch (error) {
      notify(String(error));
    }
  }

  return (
    <div className="page narrow-page">
      <header className="page-header">
        <div>
          <p className="eyebrow">SERVICE SETTINGS</p>
          <h1>翻译服务</h1>
          <p>默认使用免 Key 的 Google 网页翻译，也可以切换 DeepSeek / OpenAI 或 DeepL。</p>
        </div>
      </header>
      <section className="settings-layout">
        <div className="card settings-card">
          <div className="section-heading">
            <Sparkles />
            <div>
              <h2>翻译提供商</h2>
              <p>{preset.description}</p>
            </div>
          </div>
          <label>
            提供商
            <select
              value={value.provider}
              onChange={(event) => {
                setShow(false);
                setCredentialStatus("");
                onProviderChange(event.target.value as Provider);
              }}
            >
              {Object.entries(providerOptions).map(([id, item]) => (
                <option value={id} key={id}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        {preset.supportsGlossary && (
          <div className="card settings-card">
            <div className="section-heading">
              <BookOpen />
              <div>
                <h2>Minecraft 与模组词表</h2>
                <p>首次运行生成可编辑的默认 JSON，也可以换成自己的词表文件。</p>
              </div>
            </div>
            <label className="option-row">
              <span>
                <strong>启用术语保护</strong>
                <small>锁定常见模组名、物品、方块、机器与玩法术语，避免被模型或网页翻译误解。</small>
              </span>
              <input
                type="checkbox"
                checked={value.glossary_enabled}
                onChange={(event) => onChange({ ...value, glossary_enabled: event.target.checked })}
              />
            </label>
            <label className="glossary-path-field">
              词表文件路径
              <div className="glossary-path-control">
                <input
                  value={value.glossary_path}
                  onChange={(event) => onChange({ ...value, glossary_path: event.target.value })}
                  placeholder="选择 minecraft_glossary.json"
                />
                <button className="secondary" type="button" onClick={chooseGlossary}>
                  <FolderOpen />选择文件
                </button>
                <button className="text-button" type="button" onClick={resetGlossary}>
                  <RefreshCw />使用默认文件
                </button>
              </div>
              <small>可以直接编辑这个 JSON 文件；保存设置时会校验格式，内容变化后自动使用新的缓存空间。</small>
            </label>
            <div className="security-note">
              <ShieldCheck />
              <span>
                {value.glossary_enabled
                  ? "词表已启用 · 按文件内容隔离缓存"
                  : "词表未启用 · 使用提供商原始翻译结果"}
              </span>
            </div>
          </div>
        )}
        {needsCredential && (
          <div className="card settings-card">
            <div className="section-heading">
              <KeyRound />
              <div>
                <h2>服务凭证</h2>
                <p>普通设置不会访问钥匙串；只有查看、修改或实际翻译需要 Key 时才按需读取。</p>
              </div>
            </div>
            <label>
              {preset.credentialLabel}
              <div className="input-with-action">
                <input
                  type={show ? "text" : "password"}
                  value={value.api_key}
                  onChange={(event) => changeApiKey(event.target.value)}
                  placeholder="不会自动读取；输入新值可替换已保存的 Key"
                />
                <button onClick={toggleCredential} aria-label={show ? "隐藏密钥" : "查看已保存的密钥"}>
                  {show ? <EyeOff /> : <Eye />}
                </button>
              </div>
            </label>
            <div className="security-note">
              <ShieldCheck />
              <span>{credentialStatus || "钥匙串尚未访问"}</span>
            </div>
          </div>
        )}
        {preset.configuration === "deepl" && (
          <div className="card settings-card">
            <div className="section-heading">
              <Languages />
              <div>
                <h2>DeepL API 配置</h2>
                <p>Free 账户使用 api-free.deepl.com；Pro 账户可改为 api.deepl.com。</p>
              </div>
            </div>
            <label>
              接口地址
              <input
                value={value.base_url}
                onChange={(event) => update("base_url", event.target.value)}
                placeholder="https://api-free.deepl.com"
              />
            </label>
          </div>
        )}
        {preset.configuration === "openai" && (
          <div className="card settings-card">
            <div className="section-heading">
              <Sparkles />
              <div>
                <h2>DeepSeek / OpenAI 模型配置</h2>
                <p>仅在 DeepSeek / OpenAI 兼容模式下使用。</p>
              </div>
            </div>
            <div className="field-grid">
              <label>
                接口地址
                <input value={value.base_url} onChange={(event) => update("base_url", event.target.value)} />
              </label>
              <label>
                模型名称
                <input value={value.model} onChange={(event) => update("model", event.target.value)} />
              </label>
            </div>
            <label>
              翻译要求
              <textarea rows={5} value={value.style} onChange={(event) => update("style", event.target.value)} />
            </label>
          </div>
        )}
        {preset.supportsTaskParameters && (
          <div className="card settings-card">
            <div className="section-heading">
              <Settings />
              <div>
                <h2>任务参数</h2>
                <p>控制 API 模式下的批处理量和并发请求数。</p>
              </div>
            </div>
            <div className="field-grid">
              <label>
                每批条目
                <input
                  value={value.batch_size}
                  onChange={(event) => update("batch_size", event.target.value)}
                  placeholder="auto"
                />
                <small>不确定时保留 auto</small>
              </label>
              <label>
                并发请求
                <input
                  value={value.concurrency}
                  onChange={(event) => update("concurrency", event.target.value)}
                  placeholder="auto"
                />
                <small>网络不稳定时可手动设为 2–4</small>
              </label>
            </div>
          </div>
        )}
        <div className="card settings-card diagnostics-card">
          <div className="section-heading">
            <FileSearch />
            <div>
              <h2>诊断日志</h2>
              <p>无需单独配置；前端与后端分别写入 frontend.log 和 backend.log，默认保存在应用程序旁边。</p>
            </div>
          </div>
          <div className="diagnostics-grid">
            <label>
              日志级别
              <select
                value={value.log_level}
                onChange={(event) => onChange({ ...value, log_level: event.target.value as LogLevel })}
              >
                <option value="error">Error · 仅严重错误</option>
                <option value="warn">Warn · 错误与异常</option>
                <option value="info">Info · 日常运行（推荐）</option>
                <option value="debug">Debug · 请求与批次诊断</option>
                <option value="trace">Trace · 最详细处理过程</option>
              </select>
              <small>Debug 和 Trace 适合临时排障，日志会增长得更快。</small>
            </label>
            <div className="log-location">
              <span>默认保存位置（无需配置）</span>
              <code title={logDirectory}>{logDirectory}</code>
            </div>
          </div>
          <div className="diagnostics-actions">
            <button className="secondary" type="button" onClick={openLogs}>
              <FolderOpen />打开日志目录
            </button>
            <button className="secondary" type="button" onClick={exportLogs}>
              <Archive />导出前后端日志
            </button>
            <span>两个日志分别滚动：单文件最多 5 MB，各保留最近 5 份；API Key 与授权信息不会写入。</span>
          </div>
        </div>
        <div className="settings-actions">
          <button className="primary" onClick={onSave}>
            <Save />保存设置
          </button>
          <span>修改将在下一次任务开始时生效</span>
        </div>
      </section>
    </div>
  );
}
