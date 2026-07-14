import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Archive, ArrowRight, BookOpen, Check, ChevronRight, CircleAlert, Copy, Download, Eye, EyeOff, FileSearch, FileText, FolderOpen, History, KeyRound, Languages, Moon, Play, RefreshCw, Save, Settings, ShieldCheck, Sparkles, Sun, Trash2, Upload, X } from "lucide-react";
import "./styles.css";

type View = "workbench" | "history" | "settings";
type Stage = "idle" | "scanned" | "running" | "review" | "done" | "error";
type Provider = "openai_compatible"|"deepl"|"google_web"|"deepl_web";
type LogLevel = "error"|"warn"|"info"|"debug"|"trace";
type SettingsData = { api_key:string; api_key_changed:boolean; has_api_key:boolean; credential_backend:string; provider:Provider; base_url:string; model:string; style:string; batch_size:string; concurrency:string; log_level:LogLevel; glossary_enabled:boolean; glossary_path:string };
type ScanResult = { quests_dir:string; pack_name:string; mode:"lang"|"chapters"; mode_label:string; source:string; entry_count:number; file_count:number; files:{path:string;entry_count:number}[]; estimated_batches:number };
type Report = { source_file:string; target_file:string; backup_dir:string; total_entries:number; translated_entries:number; cache_hits:number; failed_entries:string[]; warnings:Record<string,string[]>; failed_translations:Record<string,{source:string;failed:string;error?:string}> };
type Run = { id:number; pack_name:string; quests_dir:string; mode:string; model:string; style:string; total_entries:number; translated_entries:number; cache_hits:number; failed_count:number; warning_count:number; created_at:string };
type CmpDraft = { cmp_path:string; task_id?:string; total_entries:number; warning_count:number; failed_count:number };
type CmpEntry = { index:number; entry_id:string; path:string; file:string; source:string; target:string; status:string };
type Activity = { type:"message"; message:string } | { type:"translation"; entry_id:string; source:string; target:string; status:string };
type TranslationEvent = { type:"progress"|"log"|"translation_preview"|"done"|"review_ready"|"error"; task_id?:string; stage?:string; done?:number; total?:number; message?:string; entry_id?:string; source?:string; target?:string; status?:string; report?:Report; run_id?:number; cmp_path?:string; total_entries?:number; warning_count?:number; failed_count?:number };

type ProviderPreset = {
  label:string;
  description:string;
  base_url:string;
  model:string;
  credentialLabel?:string;
  supportsGlossary:boolean;
  supportsTaskParameters:boolean;
  configuration:"none"|"deepl"|"openai";
};
const providerOptions:Record<Provider,ProviderPreset>={
  google_web:{label:"Google 网页翻译（默认）",description:"无需 API Key，使用内置的大批次、低并发策略。",base_url:"https://translate.googleapis.com",model:"google-web",supportsGlossary:false,supportsTaskParameters:false,configuration:"none"},
  deepl_web:{label:"DeepL 网页翻译（实验性）",description:"无需 API Key，使用匿名网页接口与内置安全参数。",base_url:"https://oneshot-free.www.deepl.com",model:"deepl-web",supportsGlossary:false,supportsTaskParameters:false,configuration:"none"},
  deepl:{label:"DeepL 翻译 API",description:"使用 DeepL 官方 API，可配置认证密钥、接口地址和任务参数。",base_url:"https://api-free.deepl.com",model:"deepl",credentialLabel:"DeepL Authentication Key",supportsGlossary:true,supportsTaskParameters:true,configuration:"deepl"},
  openai_compatible:{label:"DeepSeek / OpenAI 兼容",description:"可配置 API Key、兼容接口、模型、翻译要求和任务参数。",base_url:"https://api.deepseek.com",model:"deepseek-chat",credentialLabel:"API Key",supportsGlossary:true,supportsTaskParameters:true,configuration:"openai"},
};
const defaults: SettingsData = { api_key:"", api_key_changed:false, has_api_key:false, credential_backend:"系统凭证管理器", provider:"google_web", base_url:"https://translate.googleapis.com", model:"google-web", style:"准确、自然地翻译为简体中文，保留 Minecraft 与模组专有名词。", batch_size:"auto", concurrency:"auto", log_level:"info", glossary_enabled:false, glossary_path:"" };

function errorText(error:unknown){return error instanceof Error?error.message:String(error)}
const note=(message:string):Activity=>({type:"message",message});
function frontendLog(level:LogLevel,event:string,message:string,context:Record<string,unknown>={}){
  return invoke("bridge",{command:"frontend-log",payload:{level,event,message,context}}).catch(error=>console.error("frontend log write failed",error));
}
async function call<T>(command:string, payload:Record<string,unknown>={}) {
  try{return await invoke<T>("bridge", { command, payload })}
  catch(error){void frontendLog("error","bridge_call_failed","前端调用后端命令失败",{command,error:errorText(error)});throw error}
}

class FrontendErrorBoundary extends React.Component<{children:React.ReactNode},{failed:boolean}>{
  state={failed:false};
  static getDerivedStateFromError(){return{failed:true}}
  componentDidCatch(error:Error,info:React.ErrorInfo){void frontendLog("error","react_render_failed","React 界面渲染失败",{error:error.message,component_stack:info.componentStack||""})}
  render(){return this.state.failed?<main className="frontend-fatal"><CircleAlert/><h1>界面出现错误</h1><p>错误已经写入 frontend.log，请重启应用后重试。</p></main>:this.props.children}
}

function QuestMark({compact=false}:{compact?:boolean}) {
  return <div className={`brand-mark ${compact?"compact":""}`} aria-hidden="true"><span/><span/><span/></div>;
}

function App() {
  const [view,setView]=useState<View>("workbench"); const [stage,setStage]=useState<Stage>("idle");
  const [theme,setTheme]=useState<"light"|"dark">(()=>localStorage.theme==="dark"?"dark":"light");
  const [settings,setSettings]=useState<SettingsData>(defaults); const [scan,setScan]=useState<ScanResult|null>(null);
  const [selectedPath,setSelectedPath]=useState(""); const [busy,setBusy]=useState(false); const [progress,setProgress]=useState(0); const [progressDetail,setProgressDetail]=useState<{done:number;total:number}>({done:0,total:0});
  const [logs,setLogs]=useState<Activity[]>([]); const [report,setReport]=useState<Report|null>(null); const [runs,setRuns]=useState<Run[]>([]);
  const [toast,setToast]=useState(""); const [confirm,setConfirm]=useState(false); const [cmpDraft,setCmpDraft]=useState<CmpDraft|null>(null); const [cmpEntries,setCmpEntries]=useState<CmpEntry[]>([]); const [reviewPrompt,setReviewPrompt]=useState(false);

  useEffect(()=>{ document.documentElement.dataset.theme=theme; localStorage.theme=theme; void frontendLog("debug","theme_applied","界面主题已应用",{theme}); },[theme]);
  useEffect(()=>{
    void frontendLog("info","frontend_started","前端界面已启动",{language:navigator.language});
    const onError=(event:ErrorEvent)=>void frontendLog("error","window_error","前端发生未捕获错误",{error:event.message,source:event.filename,line:event.lineno,column:event.colno});
    const onRejection=(event:PromiseRejectionEvent)=>void frontendLog("error","unhandled_rejection","前端 Promise 未处理异常",{error:errorText(event.reason)});
    window.addEventListener("error",onError);window.addEventListener("unhandledrejection",onRejection);
    return()=>{window.removeEventListener("error",onError);window.removeEventListener("unhandledrejection",onRejection)};
  },[]);
  useEffect(()=>{ call<SettingsData>("settings").then(value=>{setSettings({...value,api_key:"",api_key_changed:false});void frontendLog("info","settings_loaded","前端设置已加载",{provider:value.provider,log_level:value.log_level})}).catch(e=>notify(String(e))); },[]);
  useEffect(()=>{ const unlisten=listen<TranslationEvent>("translation-event",({payload:e})=>{
    if(e.type==="log"&&e.message) setLogs(v=>[...v.slice(-299),note(e.message!)]);
    if(e.type==="translation_preview"&&e.entry_id!==undefined&&e.source!==undefined&&e.target!==undefined) setLogs(v=>[...v.slice(-299),{type:"translation",entry_id:e.entry_id!,source:e.source!,target:e.target!,status:e.status||"translated"}]);
    if(e.type==="progress") { setProgress(e.total?Math.min(100,Math.round((e.done||0)/e.total*100)):100);setProgressDetail({done:e.done||0,total:e.total||0}); void frontendLog("trace","translation_progress","前端收到翻译进度",{task_id:e.task_id||"",done:e.done||0,total:e.total||0,stage:e.stage||""}); }
    if(e.type==="review_ready"&&e.cmp_path) { const draft={cmp_path:e.cmp_path,task_id:e.task_id,total_entries:e.total_entries||0,warning_count:e.warning_count||0,failed_count:e.failed_count||0};setBusy(false);setProgress(100);setStage("review");setCmpDraft(draft);void loadCmpEntries(draft);setReviewPrompt(true);setLogs(v=>[...v,note("API 翻译完成，已打开可编辑校对表格，尚未覆盖任务书。")]);void frontendLog("info","cmp_review_ready","CMP 校对文件已生成",draft);notify("翻译完成，请确认是否直接覆盖"); }
    if(e.type==="done"&&e.report) { setBusy(false); setProgress(100); setStage("done"); setReport(e.report); setLogs(v=>[...v,note("翻译完成，输出与备份均已写入。")]); void frontendLog("info","translation_completed","前端收到翻译完成事件",{run_id:e.run_id,total:e.report.total_entries,translated:e.report.translated_entries,failed:e.report.failed_entries.length,warnings:Object.keys(e.report.warnings).length}); notify("任务书汉化完成"); loadHistory(); }
    if(e.type==="error") { setBusy(false); setStage("error"); void frontendLog("error","translation_failed","前端收到翻译失败事件",{task_id:e.task_id||"",error:e.message||"翻译失败"}); notify(e.message||"翻译失败"); }
  }); return()=>{unlisten.then(fn=>fn())}; },[]);
  const notify=(text:string)=>{setToast(text); window.setTimeout(()=>setToast(""),3200)};
  const loadHistory=()=>call<Run[]>("history-list").then(setRuns).catch(e=>notify(String(e)));
  useEffect(()=>{if(view==="history")loadHistory()},[view]);

  function navigate(next:View){void frontendLog("debug","navigation_changed","用户切换页面",{from:view,to:next});setView(next)}
  async function chooseFolder(){ const value=await open({directory:true,multiple:false,title:"选择整合包或 FTB Quests 目录"}); if(typeof value==="string"){void frontendLog("info","folder_selected","用户选择了任务书目录",{path:value});setSelectedPath(value); await doScan(value)}else{void frontendLog("debug","folder_selection_cancelled","用户取消了目录选择")} }
  async function doScan(path=selectedPath){ if(!path.trim()){void frontendLog("warn","scan_rejected","扫描未开始：目录为空");return notify("请先选择整合包目录")} setBusy(true); setReport(null); setCmpDraft(null); setCmpEntries([]); setReviewPrompt(false); void frontendLog("info","scan_started","用户开始扫描任务书",{path}); try { const result=await call<ScanResult>("scan",{path,batch_size:settings.batch_size}); setScan(result); setSelectedPath(result.quests_dir); setStage("scanned"); setProgress(0); setLogs([note(`已找到 ${result.entry_count} 条可翻译文本。`),note(`源目录：${result.source}`)]); void frontendLog("info","scan_completed","前端已展示扫描结果",{mode:result.mode,entries:result.entry_count,files:result.file_count}); } catch(e){setStage("error");notify(String(e))} finally{setBusy(false)} }
  async function beginTranslation(){setConfirm(false);if(!scan){void frontendLog("warn","translation_rejected","翻译未开始：没有扫描结果");return}setBusy(true);setStage("running");setProgress(0);setProgressDetail({done:0,total:0});setLogs([note("正在启动安全翻译任务…")]);void frontendLog("info","translation_started","用户确认开始翻译",{quests_dir:scan.quests_dir,provider:settings.provider});try{await invoke("start_translation",{payload:{quests_dir:scan.quests_dir,...settings}})}catch(e){void frontendLog("error","translation_start_failed","启动翻译命令失败",{error:errorText(e)});setBusy(false);setStage("error");notify(String(e))}}
  async function retryRateLimited(){if(!scan||!cmpDraft)return;const count=cmpEntries.filter(entry=>entry.status==="rate_limited").length;if(!count)return notify("当前没有可重试的限流条目");try{await call("cmp-save-edits",{cmp_path:cmpDraft.cmp_path,entries:cmpEntries});setBusy(true);setStage("running");setProgress(0);setProgressDetail({done:0,total:count});setLogs(v=>[...v,note(`正在重新调用翻译接口处理 ${count} 条限流内容…`)]);void frontendLog("info","rate_limited_retry_started","用户重试限流条目",{task_id:cmpDraft.task_id||"",cmp_path:cmpDraft.cmp_path,count});await invoke("start_translation",{payload:{quests_dir:scan.quests_dir,retry_cmp_path:cmpDraft.cmp_path,...settings}})}catch(e){setBusy(false);setStage("review");void frontendLog("error","rate_limited_retry_failed","限流条目重试启动失败",{task_id:cmpDraft.task_id||"",error:errorText(e)});notify(String(e))}}
  async function loadCmpEntries(draft:CmpDraft){try{const result=await call<{entries:CmpEntry[]}>("cmp-review",{cmp_path:draft.cmp_path});setCmpEntries(result.entries)}catch(e){notify(String(e))}}
  async function applyCmp(){if(!scan||!cmpDraft)return;setReviewPrompt(false);setBusy(true);setStage("running");setLogs(v=>[...v,note("正在校验校对表格、创建备份并覆盖任务书…")]);void frontendLog("info","cmp_apply_started","用户确认校对表格并应用 CMP",{task_id:cmpDraft.task_id||"",cmp_path:cmpDraft.cmp_path,entries:cmpEntries.length});try{if(cmpEntries.length)await call("cmp-save-edits",{cmp_path:cmpDraft.cmp_path,entries:cmpEntries});const result=await call<{report:Report;run_id:number;task_id:string}>("cmp-apply",{cmp_path:cmpDraft.cmp_path,quests_dir:scan.quests_dir});setBusy(false);setProgress(100);setStage("done");setReport(result.report);setLogs(v=>[...v,note("校对表格已通过校验，翻译结果已写入任务书。")]);void frontendLog("info","cmp_applied","CMP 已应用",{task_id:result.task_id,run_id:result.run_id,cmp_path:cmpDraft.cmp_path});notify("任务书汉化完成");loadHistory()}catch(e){void frontendLog("warn","cmp_apply_failed","CMP 校验或应用失败",{task_id:cmpDraft.task_id||"",cmp_path:cmpDraft.cmp_path,error:errorText(e)});setBusy(false);setStage("review");notify(String(e))}}
  async function openCmp(){if(!cmpDraft)return;try{await call("cmp-open",{cmp_path:cmpDraft.cmp_path});void frontendLog("info","cmp_opened","用户打开了 CMP 校对文件",{task_id:cmpDraft.task_id||"",cmp_path:cmpDraft.cmp_path})}catch(e){notify(String(e))}}
  async function exportCmp(){if(!cmpDraft)return;const target=await save({title:"导出 CMP 校对文件",defaultPath:"ftb-translation-review.cmp",filters:[{name:"FTB CMP 校对文件",extensions:["cmp"]}]});if(!target){void frontendLog("debug","cmp_export_cancelled","用户取消另存 CMP",{task_id:cmpDraft.task_id||""});return}try{await call("cmp-export",{cmp_path:cmpDraft.cmp_path,path:target});void frontendLog("info","cmp_exported","用户另存了 CMP 校对文件",{task_id:cmpDraft.task_id||"",path:target});notify("CMP 校对文件已导出")}catch(e){notify(String(e))}}
  async function chooseCmp(){const value=await open({multiple:false,directory:false,title:"选择 CMP 校对文件",filters:[{name:"FTB CMP 校对文件",extensions:["cmp"]}]});if(typeof value!=="string"){void frontendLog("debug","cmp_selection_cancelled","用户取消选择 CMP 校对文件");return}const draft=cmpDraft?{...cmpDraft,cmp_path:value,task_id:undefined}:{cmp_path:value,total_entries:scan?.entry_count||0,warning_count:0,failed_count:0};setCmpDraft(draft);setStage("review");setReviewPrompt(false);await loadCmpEntries(draft);void frontendLog("info","cmp_selected","用户选择了 CMP 校对文件",{cmp_path:value});notify("已打开 CMP 校对表格")}
  function reviewCmp(){if(cmpDraft)void frontendLog("info","cmp_manual_review_selected","用户选择先人工校对 CMP",{task_id:cmpDraft.task_id||"",cmp_path:cmpDraft.cmp_path});setReviewPrompt(false)}
  async function saveSettings(){try{const r=await call<{credential_backend:string;glossary_path:string}>("save-settings",settings);setSettings(v=>({...v,api_key:"",api_key_changed:false,has_api_key:v.api_key_changed?!!v.api_key.trim():v.has_api_key,credential_backend:r.credential_backend,glossary_path:r.glossary_path}));void frontendLog("info","settings_saved","用户保存了设置",{provider:settings.provider,log_level:settings.log_level,glossary_enabled:settings.glossary_enabled});notify("设置已保存")}catch(e){notify(String(e))}}
  function changeProvider(provider:Provider){void frontendLog("info","provider_changed","用户切换翻译提供商",{from:settings.provider,to:provider});const preset=providerOptions[provider];setSettings(v=>({...v,provider,api_key:"",api_key_changed:false,has_api_key:false,base_url:preset.base_url,model:preset.model,glossary_enabled:preset.supportsGlossary?v.glossary_enabled:false,batch_size:preset.supportsTaskParameters?v.batch_size:"auto",concurrency:preset.supportsTaskParameters?v.concurrency:"auto"}))}
  const warningCount=report?Object.keys(report.warnings).length:0;

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><QuestMark/><div><strong>FTB Translater</strong><span>任务书汉化工作台</span></div></div>
      <nav aria-label="主导航">
        <Nav active={view==="workbench"} icon={<BookOpen/>} label="翻译工作台" onClick={()=>navigate("workbench")}/>
        <Nav active={view==="history"} icon={<History/>} label="翻译历史" onClick={()=>navigate("history")} badge={runs.length||undefined}/>
        <Nav active={view==="settings"} icon={<Settings/>} label="服务设置" onClick={()=>navigate("settings")}/>
      </nav>
      <button className="theme-toggle" onClick={()=>setTheme(theme==="light"?"dark":"light")}>{theme==="light"?<Moon/>:<Sun/>}<span>{theme==="light"?"切换深色":"切换浅色"}</span></button>
    </aside>
    <main className="main-area">
      {view==="workbench"&&<Workbench
        stage={stage} scan={scan} path={selectedPath} setPath={setSelectedPath}
        busy={busy} progress={progress} progressDetail={progressDetail} logs={logs} report={report} warnings={warningCount} cmpDraft={cmpDraft} cmpEntries={cmpEntries} setCmpEntries={setCmpEntries}
        onChoose={chooseFolder} onScan={()=>doScan()} onTranslate={()=>setConfirm(true)} onSettings={()=>navigate("settings")}
        onOpenCmp={openCmp} onExportCmp={exportCmp} onChooseCmp={chooseCmp} onApplyCmp={applyCmp} onRetryRateLimited={retryRateLimited}
      />}
      {view==="settings"&&<SettingsPage value={settings} onChange={setSettings} onProviderChange={changeProvider} onSave={saveSettings} notify={notify}/>}
      {view==="history"&&<HistoryPage runs={runs} notify={notify} reload={loadHistory}/>} 
    </main>
    {confirm&&scan&&<ConfirmDialog scan={scan} onCancel={()=>setConfirm(false)} onConfirm={beginTranslation}/>} 
    {reviewPrompt&&cmpDraft&&<CmpDecisionDialog draft={cmpDraft} onReview={reviewCmp} onApply={applyCmp}/>}
    {toast&&<div className="toast"><Check/>{toast}</div>}
  </div>
}

function Nav({active,icon,label,onClick,badge}:{active:boolean;icon:React.ReactNode;label:string;onClick:()=>void;badge?:number}){return <button className={`nav-item ${active?"active":""}`} onClick={onClick}>{icon}<span>{label}</span>{badge!==undefined&&<em>{badge}</em>}</button>}

function Workbench(p:{stage:Stage;scan:ScanResult|null;path:string;setPath:(v:string)=>void;busy:boolean;progress:number;progressDetail:{done:number;total:number};logs:Activity[];report:Report|null;warnings:number;cmpDraft:CmpDraft|null;cmpEntries:CmpEntry[];setCmpEntries:React.Dispatch<React.SetStateAction<CmpEntry[]>>;onChoose:()=>void;onScan:()=>void;onTranslate:()=>void;onSettings:()=>void;onOpenCmp:()=>void;onExportCmp:()=>void;onChooseCmp:()=>void;onApplyCmp:()=>void;onRetryRateLimited:()=>void}){
  const steps=[{key:"idle",label:"选择任务书"},{key:"running",label:"API 翻译"},{key:"review",label:"校对 CMP"},{key:"done",label:"完成写回"}]; const index={idle:0,scanned:0,running:1,review:2,done:3,error:p.scan?1:0}[p.stage]; const rateLimitedCount=p.cmpEntries.filter(entry=>entry.status==="rate_limited").length;
  return <div className="page workbench-page">
    <header className="page-header"><div><p className="eyebrow">TRANSLATION WORKBENCH</p><h1>把任务书带给中文玩家</h1><p>先生成可人工校对的 CMP 文件，确认后才会备份并写回。</p></div><div className={`status-pill ${p.stage}`}><span/>{p.stage==="running"?"正在处理":p.stage==="review"?"等待校对":p.stage==="done"?"本次完成":p.stage==="error"?"需要处理":p.stage==="scanned"?"等待开始":"准备就绪"}</div></header>
    <section className="quest-chain" aria-label="汉化进度">{steps.map((s,i)=><React.Fragment key={s.key}><div className={`quest-step ${i<=index?"active":""} ${i<index?"complete":""}`}><i>{i<index?<Check/>:i+1}</i><span>{s.label}</span></div>{i<3&&<div className={`quest-link ${i<index?"active":""}`}><span/></div>}</React.Fragment>)}</section>
    <div className="workspace-grid">
      <section className="card source-card"><div className="card-title"><div className="icon-tile blue"><FolderOpen/></div><div><h2>任务书位置</h2><p>整合包根目录或 quests、lang、chapters 目录都可以</p></div></div><div className="path-control"><input value={p.path} onChange={e=>p.setPath(e.target.value)} placeholder="选择一个整合包目录…" onKeyDown={e=>e.key==="Enter"&&p.onScan()}/><button className="secondary" onClick={p.onChoose}><FolderOpen/>选择目录</button></div>{p.scan?<ScanSummary scan={p.scan}/>:<div className="drop-hint" onClick={p.onChoose}><FileSearch/><div><strong>从扫描开始</strong><span>我们会自动判断任务书格式，不会在扫描时改动文件。</span></div><ChevronRight/></div>}</section>
      <aside className="card action-card"><p className="eyebrow">NEXT ACTION</p>{p.stage==="idle"||p.stage==="error"&&!p.scan?<><h2>先找到任务书</h2><p>扫描只读取目录结构和可翻译条目，不会覆盖任何文件。</p><button className="primary wide" disabled={p.busy} onClick={p.onScan}>{p.busy?<RefreshCw className="spin"/>:<FileSearch/>}扫描任务书</button></>:p.stage==="scanned"?<><h2>生成校对文件</h2><p>API 翻译完成后先生成 CMP，确认之前不会修改任务书。</p><button className="primary wide" onClick={p.onTranslate}><Play/>开始翻译<ArrowRight/></button><button className="text-button" onClick={p.onChooseCmp}><Upload/>选择已有 CMP</button><button className="text-button" onClick={p.onSettings}><Settings/>检查翻译设置</button></>:p.stage==="running"?<><h2>{p.progress<100?"正在翻译":"正在校验并写回"}</h2><p>处理完成前请保留窗口，程序不会写入未经确认的译文。</p><div className="progress-number"><strong>{p.progress}</strong><span>%</span></div><div className="progress-detail">{p.progressDetail.total?`已处理 ${p.progressDetail.done} / ${p.progressDetail.total} 个翻译单元`:"正在准备翻译单元…"}</div><div className="progress-track"><span style={{width:`${p.progress}%`}}/></div></>:p.stage==="review"?<><h2>等待人工确认</h2><p>可以编辑 CMP 右侧中文，完成后再应用并覆盖任务书。</p><div className="result-mini amber"><FileText/><span>CMP 尚未写回</span></div><button className="primary wide" onClick={p.onApplyCmp}><ShieldCheck/>校验并覆盖</button></>:<><h2>汉化已经写入</h2><p>{p.warnings?`有 ${p.warnings} 条内容建议人工确认，其余内容已完成。`:"格式检查全部通过，可以进入游戏查看效果。"}</p><div className="result-mini"><Check/><span>备份已创建</span></div><button className="secondary wide" onClick={p.onChoose}>翻译另一个整合包</button></>}</aside>
    </div>
    {p.stage==="review"&&p.cmpDraft&&<><div className="rate-limit-retry-bar"><div><RefreshCw/><span><strong>{rateLimitedCount} 条因接口限流未翻译</strong><small>只重新请求这一批，其他译文和人工修改保持不变。</small></span></div><button className="secondary" disabled={p.busy||rateLimitedCount===0} onClick={p.onRetryRateLimited}><RefreshCw/>重试限流项</button></div><CmpReviewTable draft={p.cmpDraft} entries={p.cmpEntries} setEntries={p.setCmpEntries} onOpen={p.onOpenCmp} onExport={p.onExportCmp} onChoose={p.onChooseCmp} onApply={p.onApplyCmp}/></>}
    {(p.logs.length>0||p.report)&&<section className="lower-grid"><ActivityFeed entries={p.logs}/>{p.report&&<div className="card report-card"><div className="card-title compact"><div><h2>本次结果</h2><p>{p.report.translated_entries} / {p.report.total_entries} 条已处理</p></div></div><Metric label="缓存命中" value={p.report.cache_hits}/><Metric label="需要检查" value={p.warnings} warn={p.warnings>0}/><Metric label="翻译失败" value={p.report.failed_entries.length} warn={p.report.failed_entries.length>0}/><div className="backup-path"><Archive/><span title={p.report.backup_dir}>{p.report.backup_dir}</span><button onClick={()=>navigator.clipboard.writeText(p.report!.backup_dir)} aria-label="复制备份路径"><Copy/></button></div></div>}</section>}
  </div>
}

function ActivityFeed({entries}:{entries:Activity[]}){const box=useRef<HTMLDivElement>(null);useEffect(()=>{box.current?.scrollTo({top:box.current.scrollHeight,behavior:"smooth"})},[entries.length]);return <div className="card log-card activity-card"><div className="card-title compact"><div><h2>实时翻译记录</h2><p>自动跟随最新内容；英文与中文仅显示在本次界面，不写入诊断日志。</p></div><span className="live-dot">自动滚动</span></div><div className="log-list activity-list" ref={box}>{entries.map((entry,index)=>entry.type==="message"?<div className="activity-message" key={`m-${index}`}><span>{String(index+1).padStart(3,"0")}</span><p>{entry.message}</p></div>:<article className="activity-translation" key={`t-${index}`}><header><code>{entry.entry_id}</code><span className={entry.status==="translated"?"status-ok":"status-review"}>{entry.status==="translated"?"已翻译":"需校对"}</span></header><div><p>{entry.source}</p><ArrowRight/><strong>{entry.target}</strong></div></article>)}</div></div>}

function CmpReviewTable({draft,entries,setEntries,onOpen,onExport,onChoose,onApply}:{draft:CmpDraft;entries:CmpEntry[];setEntries:React.Dispatch<React.SetStateAction<CmpEntry[]>>;onOpen:()=>void;onExport:()=>void;onChoose:()=>void;onApply:()=>void}){const [query,setQuery]=useState("");const [status,setStatus]=useState("all");const [page,setPage]=useState(0);const pageSize=60;const filtered=useMemo(()=>entries.filter(entry=>(status==="all"||entry.status===status)&&`${entry.file} ${entry.entry_id} ${entry.source} ${entry.target}`.toLowerCase().includes(query.toLowerCase())),[entries,query,status]);const rows=filtered.slice(page*pageSize,page*pageSize+pageSize);const pages=Math.max(1,Math.ceil(filtered.length/pageSize));useEffect(()=>setPage(0),[query,status]);function update(index:number,target:string){setEntries(items=>items.map(item=>item.index===index?{...item,target}:item))}function label(value:string){return value==="translated"?"已翻译":value==="rate_limited"?"接口限流（可重试）":value==="request_failed"?"接口失败":value==="format_guard"?"格式需检查":value==="review"?"需检查":"原文保留"}return <section className="review-table card"><header className="review-table-header"><div><p className="eyebrow">TRANSLATION REVIEW</p><h2>在表格中校对，然后一次覆盖</h2><p>直接修改中文列；点击确认后会校验格式、创建备份，并统一写入任务书。</p></div><div className="review-table-summary"><strong>{entries.length}</strong><span>条译文</span></div></header><div className="review-table-toolbar"><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索英文、中文、文件或条目…"/><label className="status-filter"><span>状态</span><select value={status} onChange={e=>setStatus(e.target.value)}><option value="all">全部状态</option><option value="translated">已翻译</option><option value="rate_limited">接口限流（可重试）</option><option value="request_failed">接口失败</option><option value="format_guard">格式需检查</option><option value="review">需检查</option><option value="unchanged">原文保留</option></select></label><span>{filtered.length} 条匹配</span><button className="secondary" onClick={onOpen}><FileText/>外部编辑 CMP</button><button className="secondary" onClick={onExport}><Download/>导出</button><button className="secondary" onClick={onChoose}><Upload/>导入 CMP</button><button className="primary" onClick={onApply}><ShieldCheck/>确认并覆盖</button></div><div className="review-table-scroll"><table><thead><tr><th>来源</th><th>英文原文</th><th>中文译文（可编辑）</th><th>状态</th></tr></thead><tbody>{rows.map(entry=><tr key={entry.index}><td><code title={`${entry.file}\n${entry.entry_id}`}>{entry.entry_id}</code><small>{entry.file}</small></td><td><p>{entry.source}</p></td><td><textarea aria-label={`第 ${entry.index+1} 条中文译文`} value={entry.target} onChange={e=>update(entry.index,e.target.value)} rows={Math.min(5,Math.max(2,entry.target.split("\n").length))}/></td><td><span className={entry.status==="translated"?"status-ok":"status-review"}>{label(entry.status)}</span></td></tr>)}</tbody></table></div><footer className="review-table-footer"><span>第 {Math.min(page*pageSize+1,filtered.length)}–{Math.min((page+1)*pageSize,filtered.length)} 条，共 {filtered.length} 条</span><div><button className="secondary" disabled={page===0} onClick={()=>setPage(value=>value-1)}>上一页</button><span>{page+1} / {pages}</span><button className="secondary" disabled={page>=pages-1} onClick={()=>setPage(value=>value+1)}>下一页</button></div></footer></section>}

function CmpDecisionDialog({draft,onReview,onApply}:{draft:CmpDraft;onReview:()=>void;onApply:()=>void}){return <div className="modal-backdrop"><div className="modal" role="dialog" aria-modal="true"><div className="modal-icon"><FileText/></div><p className="eyebrow">TRANSLATION READY</p><h2>API 翻译完成，要直接覆盖吗？</h2><p>已经生成包含英文 → 中文对照的 CMP 文件。选择“否”可先人工修改；选择“是”会立即校验、创建备份并写回任务书。</p><div className="confirm-target"><span>CMP 校对文件</span><strong title={draft.cmp_path}>{draft.cmp_path.split(/[\\/]/).pop()}</strong></div><div className="modal-actions"><button className="secondary" onClick={onReview}>否，人工校对</button><button className="primary" onClick={onApply}><Check/>是，直接覆盖</button></div></div></div>}

function ScanSummary({scan}:{scan:ScanResult}){return <div className="scan-summary"><div className="pack-row"><div className="pack-icon"><QuestMark compact/></div><div><span>已识别整合包</span><strong>{scan.pack_name||"FTB Quests"}</strong></div><span className="mode-badge">{scan.mode_label}</span></div><div className="scan-stats"><div><strong>{scan.entry_count.toLocaleString()}</strong><span>可翻译条目</span></div><div><strong>{scan.file_count}</strong><span>{scan.mode==="lang"?"语言文件":"章节文件"}</span></div><div><strong>{scan.estimated_batches}</strong><span>预计请求批次</span></div></div><div className="scan-files">{scan.files.map(file=><div key={file.path}><code>{file.path}</code><span>{file.entry_count} 条</span></div>)}</div><p className="mono-path">{scan.source}</p></div>}
function Metric({label,value,warn=false}:{label:string;value:number;warn?:boolean}){return <div className="metric"><span>{label}</span><strong className={warn?"warn":""}>{value}</strong></div>}

function SettingsPage({value,onChange,onProviderChange,onSave,notify}:{value:SettingsData;onChange:(v:SettingsData)=>void;onProviderChange:(v:Provider)=>void;onSave:()=>void;notify:(v:string)=>void}) {
  const [show,setShow]=useState(false);
  const [credentialStatus,setCredentialStatus]=useState("");
  const [logDirectory,setLogDirectory]=useState("正在读取应用目录…");
  const update=(k:keyof SettingsData,v:string)=>onChange({...value,[k]:v});
  const preset=providerOptions[value.provider];
  const needsCredential=!!preset.credentialLabel;

  useEffect(()=>{call<{directory:string;backend:string;frontend:string}>("logs-info").then(result=>setLogDirectory(`${result.directory} · ${result.backend} / ${result.frontend}`)).catch(error=>setLogDirectory(String(error)))},[]);

  async function toggleCredential(){
    if(show){setShow(false);return}
    if(value.api_key||value.api_key_changed){setShow(true);return}
    setCredentialStatus("正在读取钥匙串…");
    try{
      const saved=await call<{api_key:string;has_api_key:boolean}>("provider-credential",{provider:value.provider});
      onChange({...value,api_key:saved.api_key,api_key_changed:false,has_api_key:saved.has_api_key});
      setShow(true);
      setCredentialStatus(saved.has_api_key?"已加载到本次应用会话":"钥匙串中没有当前服务的 Key");
      void frontendLog("info","credential_viewed","用户查看了当前服务的凭证状态",{provider:value.provider,has_api_key:saved.has_api_key});
    }catch(e){setCredentialStatus(String(e))}
  }

  function changeApiKey(api_key:string){
    onChange({...value,api_key,api_key_changed:true,has_api_key:!!api_key.trim()});
    setCredentialStatus(api_key.trim()?"新 Key 待保存":"保存后将删除当前服务的 Key");
  }

  async function chooseGlossary(){
    const path=await open({multiple:false,directory:false,title:"选择 Minecraft 词表 JSON",filters:[{name:"JSON 词表",extensions:["json"]}]});
    if(typeof path==="string"){onChange({...value,glossary_path:path});void frontendLog("info","glossary_selected","用户选择了词表文件",{path})}
  }

  async function resetGlossary(){
    const result=await call<{path:string}>("default-glossary");
    onChange({...value,glossary_path:result.path});
    void frontendLog("info","glossary_reset","用户恢复了默认词表路径",{path:result.path});
  }

  async function openLogs(){
    try{await call("logs-open");void frontendLog("info","logs_opened","用户打开了日志目录");notify("已打开日志目录")}catch(error){notify(String(error))}
  }

  async function exportLogs(){
    const target=await save({title:"导出诊断日志",defaultPath:"ftb-translater-logs.zip",filters:[{name:"ZIP 压缩包",extensions:["zip"]}]});
    if(!target)return;
    try{await call("logs-export",{path:target});void frontendLog("info","logs_exported","用户导出了前后端诊断日志",{path:target});notify("诊断日志已导出")}catch(error){notify(String(error))}
  }

  return <div className="page narrow-page">
    <header className="page-header"><div><p className="eyebrow">SERVICE SETTINGS</p><h1>翻译服务</h1><p>默认使用免 Key 的 Google 网页翻译，也可以切换 DeepSeek / OpenAI 或 DeepL。</p></div></header>
    <section className="settings-layout">
      <div className="card settings-card">
        <div className="section-heading"><Sparkles/><div><h2>翻译提供商</h2><p>{preset.description}</p></div></div>
        <label>提供商<select value={value.provider} onChange={e=>{setShow(false);setCredentialStatus("");onProviderChange(e.target.value as Provider)}}>{Object.entries(providerOptions).map(([id,item])=><option value={id} key={id}>{item.label}</option>)}</select></label>
      </div>
      {preset.supportsGlossary&&<div className="card settings-card">
        <div className="section-heading"><BookOpen/><div><h2>Minecraft 与模组词表</h2><p>首次运行生成可编辑的默认 JSON，也可以换成自己的词表文件。</p></div></div>
        <label className="option-row"><span><strong>启用术语保护</strong><small>锁定常见模组名、物品、方块、机器与玩法术语，避免被模型或网页翻译误解。</small></span><input type="checkbox" checked={value.glossary_enabled} onChange={e=>onChange({...value,glossary_enabled:e.target.checked})}/></label>
        <label className="glossary-path-field">词表文件路径<div className="glossary-path-control"><input value={value.glossary_path} onChange={e=>onChange({...value,glossary_path:e.target.value})} placeholder="选择 minecraft_glossary.json"/><button className="secondary" type="button" onClick={chooseGlossary}><FolderOpen/>选择文件</button><button className="text-button" type="button" onClick={resetGlossary}><RefreshCw/>使用默认文件</button></div><small>可以直接编辑这个 JSON 文件；保存设置时会校验格式，内容变化后自动使用新的缓存空间。</small></label>
        <div className="security-note"><ShieldCheck/><span>{value.glossary_enabled?"词表已启用 · 按文件内容隔离缓存":"词表未启用 · 使用提供商原始翻译结果"}</span></div>
      </div>}
      {needsCredential&&<div className="card settings-card">
        <div className="section-heading"><KeyRound/><div><h2>服务凭证</h2><p>普通设置不会访问钥匙串；只有查看、修改或实际翻译需要 Key 时才按需读取。</p></div></div>
        <label>{preset.credentialLabel}<div className="input-with-action"><input type={show?"text":"password"} value={value.api_key} onChange={e=>changeApiKey(e.target.value)} placeholder="不会自动读取；输入新值可替换已保存的 Key"/><button onClick={toggleCredential} aria-label={show?"隐藏密钥":"查看已保存的密钥"}>{show?<EyeOff/>:<Eye/>}</button></div></label>
        <div className="security-note"><ShieldCheck/><span>{credentialStatus||"钥匙串尚未访问"}</span></div>
      </div>}
      {preset.configuration==="deepl"&&<div className="card settings-card">
        <div className="section-heading"><Languages/><div><h2>DeepL API 配置</h2><p>Free 账户使用 api-free.deepl.com；Pro 账户可改为 api.deepl.com。</p></div></div>
        <label>接口地址<input value={value.base_url} onChange={e=>update("base_url",e.target.value)} placeholder="https://api-free.deepl.com"/></label>
      </div>}
      {preset.configuration==="openai"&&<div className="card settings-card">
        <div className="section-heading"><Sparkles/><div><h2>DeepSeek / OpenAI 模型配置</h2><p>仅在 DeepSeek / OpenAI 兼容模式下使用。</p></div></div>
        <div className="field-grid"><label>接口地址<input value={value.base_url} onChange={e=>update("base_url",e.target.value)}/></label><label>模型名称<input value={value.model} onChange={e=>update("model",e.target.value)}/></label></div>
        <label>翻译要求<textarea rows={5} value={value.style} onChange={e=>update("style",e.target.value)}/></label>
      </div>}
      {preset.supportsTaskParameters&&<div className="card settings-card">
        <div className="section-heading"><Settings/><div><h2>任务参数</h2><p>控制 API 模式下的批处理量和并发请求数。</p></div></div>
        <div className="field-grid"><label>每批条目<input value={value.batch_size} onChange={e=>update("batch_size",e.target.value)} placeholder="auto"/><small>不确定时保留 auto</small></label><label>并发请求<input value={value.concurrency} onChange={e=>update("concurrency",e.target.value)} placeholder="auto"/><small>网络不稳定时可手动设为 2–4</small></label></div>
      </div>}
      <div className="card settings-card diagnostics-card">
        <div className="section-heading"><FileSearch/><div><h2>诊断日志</h2><p>无需单独配置；前端与后端分别写入 frontend.log 和 backend.log，默认保存在应用程序旁边。</p></div></div>
        <div className="diagnostics-grid">
          <label>日志级别<select value={value.log_level} onChange={e=>onChange({...value,log_level:e.target.value as LogLevel})}><option value="error">Error · 仅严重错误</option><option value="warn">Warn · 错误与异常</option><option value="info">Info · 日常运行（推荐）</option><option value="debug">Debug · 请求与批次诊断</option><option value="trace">Trace · 最详细处理过程</option></select><small>Debug 和 Trace 适合临时排障，日志会增长得更快。</small></label>
          <div className="log-location"><span>默认保存位置（无需配置）</span><code title={logDirectory}>{logDirectory}</code></div>
        </div>
        <div className="diagnostics-actions"><button className="secondary" type="button" onClick={openLogs}><FolderOpen/>打开日志目录</button><button className="secondary" type="button" onClick={exportLogs}><Archive/>导出前后端日志</button><span>两个日志分别滚动：单文件最多 5 MB，各保留最近 5 份；API Key 与授权信息不会写入。</span></div>
      </div>
      <div className="settings-actions"><button className="primary" onClick={onSave}><Save/>保存设置</button><span>修改将在下一次任务开始时生效</span></div>
    </section>
  </div>
}

function HistoryPage({runs,notify,reload}:{runs:Run[];notify:(v:string)=>void;reload:()=>void}){const [query,setQuery]=useState("");const filtered=useMemo(()=>runs.filter(r=>`${r.pack_name} ${r.quests_dir}`.toLowerCase().includes(query.toLowerCase())),[runs,query]);async function remove(id:number){if(!window.confirm("删除这条历史记录？已经写入整合包的文件不会被删除。")){void frontendLog("debug","history_delete_cancelled","用户取消删除历史记录",{run_id:id});return}try{await call("history-delete",{run_id:id});void frontendLog("info","history_deleted","用户删除了历史记录",{run_id:id});reload();notify("历史记录已删除")}catch(e){notify(String(e))}}async function exportRun(r:Run){const target=await save({title:"导出汉化内容",defaultPath:`${r.pack_name||"ftb-translation"}-${r.id}.zip`,filters:[{name:"ZIP 压缩包",extensions:["zip"]}]});if(target)try{await call("history-export",{run_id:r.id,path:target});void frontendLog("info","history_exported","用户导出了翻译历史",{run_id:r.id,path:target});notify("ZIP 已导出")}catch(e){notify(String(e))}}return <div className="page"><header className="page-header history-header"><div><p className="eyebrow">TRANSLATION ARCHIVE</p><h1>翻译历史</h1><p>重新找到每一次写入、备份和可导出的汉化结果。</p></div><div className="search-box"><FileSearch/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索整合包或路径"/></div></header>{filtered.length?<div className="history-list">{filtered.map(r=><article className="history-row" key={r.id}><div className="history-date"><strong>{new Date(r.created_at).toLocaleDateString("zh-CN",{month:"short",day:"numeric"})}</strong><span>{new Date(r.created_at).toLocaleTimeString("zh-CN",{hour:"2-digit",minute:"2-digit"})}</span></div><div className="history-main"><div><h2>{r.pack_name||"未命名整合包"}</h2><span className="mode-badge">{r.mode==="lang"?"语言文件":"章节文件"}</span></div><p>{r.quests_dir}</p><div className="history-facts"><span><Check/>{r.translated_entries} 条完成</span><span className={r.warning_count?"warning":""}><CircleAlert/>{r.warning_count} 条检查</span><span>{r.model}</span></div></div><div className="history-actions"><button className="secondary" onClick={()=>exportRun(r)}><Archive/>导出</button><button className="icon-button danger" onClick={()=>remove(r.id)} aria-label="删除"><Trash2/></button></div></article>)}</div>:<div className="empty-state"><div><History/></div><h2>{query?"没有匹配的记录":"还没有翻译历史"}</h2><p>{query?"换一个整合包名称或目录关键词。":"完成第一次汉化后，结果会自动出现在这里。"}</p></div>}</div>}

function ConfirmDialog({scan,onCancel,onConfirm}:{scan:ScanResult;onCancel:()=>void;onConfirm:()=>void}){return <div className="modal-backdrop" onMouseDown={e=>e.target===e.currentTarget&&onCancel()}><div className="modal" role="dialog" aria-modal="true"><button className="modal-close" onClick={onCancel}><X/></button><div className="modal-icon"><Languages/></div><p className="eyebrow">READY TO TRANSLATE</p><h2>翻译 {scan.entry_count.toLocaleString()} 条内容并生成 CMP？</h2><p>本阶段只调用 API 并生成英文 → 中文校对文件，不会覆盖 <code>{scan.mode==="lang"?"lang":"chapters"}</code>。确认应用 CMP 时才会创建备份并写回。</p><div className="confirm-target"><span>最终写入目标</span><strong>{scan.mode==="lang"?"lang/zh_cn.snbt":"chapters/*.snbt"}</strong></div><div className="modal-actions"><button className="secondary" onClick={onCancel}>暂不开始</button><button className="primary" onClick={onConfirm}><Play/>翻译并生成 CMP</button></div></div></div>}

createRoot(document.getElementById("root")!).render(<React.StrictMode><FrontendErrorBoundary><App/></FrontendErrorBoundary></React.StrictMode>);
