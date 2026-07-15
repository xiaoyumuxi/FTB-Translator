export type Provider = "openai_compatible" | "deepl" | "google_web" | "deepl_web";

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export type SettingsData = {
  api_key: string;
  api_key_changed: boolean;
  has_api_key: boolean;
  credential_backend: string;
  provider: Provider;
  base_url: string;
  model: string;
  style: string;
  batch_size: string;
  concurrency: string;
  log_level: LogLevel;
  glossary_enabled: boolean;
  glossary_path: string;
};

export type ProviderPreset = {
  label: string;
  description: string;
  base_url: string;
  model: string;
  credentialLabel?: string;
  supportsGlossary: boolean;
  supportsTaskParameters: boolean;
  configuration: "none" | "deepl" | "openai";
};

export const providerOptions: Record<Provider, ProviderPreset> = {
  google_web: {
    label: "Google 网页翻译（默认）",
    description: "无需 API Key，使用内置的大批次、低并发策略。",
    base_url: "https://translate.googleapis.com",
    model: "google-web",
    supportsGlossary: false,
    supportsTaskParameters: false,
    configuration: "none",
  },
  deepl_web: {
    label: "DeepL 网页翻译（实验性）",
    description: "无需 API Key，使用匿名网页接口与内置安全参数。",
    base_url: "https://oneshot-free.www.deepl.com",
    model: "deepl-web",
    supportsGlossary: false,
    supportsTaskParameters: false,
    configuration: "none",
  },
  deepl: {
    label: "DeepL 翻译 API",
    description: "使用 DeepL 官方 API，可配置认证密钥、接口地址和任务参数。",
    base_url: "https://api-free.deepl.com",
    model: "deepl",
    credentialLabel: "DeepL Authentication Key",
    supportsGlossary: true,
    supportsTaskParameters: true,
    configuration: "deepl",
  },
  openai_compatible: {
    label: "DeepSeek / OpenAI 兼容",
    description: "可配置 API Key、兼容接口、模型、翻译要求和任务参数。",
    base_url: "https://api.deepseek.com",
    model: "deepseek-chat",
    credentialLabel: "API Key",
    supportsGlossary: true,
    supportsTaskParameters: true,
    configuration: "openai",
  },
};

export const defaultSettings: SettingsData = {
  api_key: "",
  api_key_changed: false,
  has_api_key: false,
  credential_backend: "系统凭证管理器",
  provider: "google_web",
  base_url: "https://translate.googleapis.com",
  model: "google-web",
  style: "准确、自然地翻译为简体中文，保留 Minecraft 与模组专有名词。",
  batch_size: "auto",
  concurrency: "auto",
  log_level: "info",
  glossary_enabled: false,
  glossary_path: "",
};
