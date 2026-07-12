import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Archive, ArrowRight, BookOpen, Check, ChevronRight, CircleAlert, Copy, Eye, EyeOff, FileSearch, FolderOpen, History, KeyRound, Languages, Moon, Play, RefreshCw, Save, Settings, ShieldCheck, Sparkles, Sun, Trash2, X } from "lucide-react";
import "./styles.css";

type View = "workbench" | "history" | "settings";
type Stage = "idle" | "scanned" | "running" | "done" | "error";
type SettingsData = { api_key:string; has_api_key:boolean; credential_backend:string; base_url:string; model:string; style:string; batch_size:string; concurrency:string };
type ScanResult = { quests_dir:string; pack_name:string; mode:"lang"|"chapters"; mode_label:string; source:string; entry_count:number; file_count:number; estimated_batches:number };
type Report = { source_file:string; target_file:string; backup_dir:string; total_entries:number; translated_entries:number; cache_hits:number; failed_entries:string[]; warnings:Record<string,string[]>; failed_translations:Record<string,{source:string;failed:string;error?:string}> };
type Run = { id:number; pack_name:string; quests_dir:string; mode:string; model:string; style:string; total_entries:number; translated_entries:number; cache_hits:number; failed_count:number; warning_count:number; created_at:string };
type TranslationEvent = { type:"progress"|"log"|"done"|"error"; stage?:string; done?:number; total?:number; message?:string; report?:Report; run_id?:number };

const defaults: SettingsData = { api_key:"", has_api_key:false, credential_backend:"系统凭证管理器", base_url:"https://api.deepseek.com", model:"deepseek-chat", style:"准确、自然地翻译为简体中文，保留 Minecraft 与模组专有名词。", batch_size:"auto", concurrency:"auto" };

async function call<T>(command:string, payload:Record<string,unknown>={}) { return invoke<T>("bridge", { command, payload }); }

function QuestMark({compact=false}:{compact?:boolean}) {
  return <div className={`brand-mark ${compact?"compact":""}`} aria-hidden="true"><span/><span/><span/></div>;
}

function App() {
  const [view,setView]=useState<View>("workbench"); const [stage,setStage]=useState<Stage>("idle");
  const [theme,setTheme]=useState<"light"|"dark">(()=>localStorage.theme==="dark"?"dark":"light");
  const [settings,setSettings]=useState<SettingsData>(defaults); const [scan,setScan]=useState<ScanResult|null>(null);
  const [selectedPath,setSelectedPath]=useState(""); const [busy,setBusy]=useState(false); const [progress,setProgress]=useState(0);
  const [logs,setLogs]=useState<string[]>([]); const [report,setReport]=useState<Report|null>(null); const [runs,setRuns]=useState<Run[]>([]);
  const [toast,setToast]=useState(""); const [confirm,setConfirm]=useState(false);

  useEffect(()=>{ document.documentElement.dataset.theme=theme; localStorage.theme=theme; },[theme]);
  useEffect(()=>{ call<SettingsData>("settings").then(setSettings).catch(e=>notify(String(e))); },[]);
  useEffect(()=>{ const unlisten=listen<TranslationEvent>("translation-event",({payload:e})=>{
    if(e.type==="log"&&e.message) setLogs(v=>[...v.slice(-99),e.message!]);
    if(e.type==="progress") { setProgress(e.total?Math.min(100,Math.round((e.done||0)/e.total*100)):100); }
    if(e.type==="done"&&e.report) { setBusy(false); setProgress(100); setStage("done"); setReport(e.report); setLogs(v=>[...v,"翻译完成，输出与备份均已写入。"]); notify("任务书汉化完成"); loadHistory(); }
    if(e.type==="error") { setBusy(false); setStage("error"); notify(e.message||"翻译失败"); }
  }); return()=>{unlisten.then(fn=>fn())}; },[]);
  const notify=(text:string)=>{setToast(text); window.setTimeout(()=>setToast(""),3200)};
  const loadHistory=()=>call<Run[]>("history-list").then(setRuns).catch(e=>notify(String(e)));
  useEffect(()=>{if(view==="history")loadHistory()},[view]);

  async function chooseFolder(){ const value=await open({directory:true,multiple:false,title:"选择整合包或 FTB Quests 目录"}); if(typeof value==="string"){setSelectedPath(value); await doScan(value)} }
  async function doScan(path=selectedPath){ if(!path.trim())return notify("请先选择整合包目录"); setBusy(true); setReport(null); try { const result=await call<ScanResult>("scan",{path,batch_size:settings.batch_size}); setScan(result); setSelectedPath(result.quests_dir); setStage("scanned"); setProgress(0); setLogs([`已找到 ${result.entry_count} 条可翻译文本。`,`源目录：${result.source}`]); } catch(e){setStage("error");notify(String(e))} finally{setBusy(false)} }
  async function beginTranslation(){setConfirm(false);if(!scan)return;setBusy(true);setStage("running");setProgress(0);setLogs(["正在启动安全翻译任务…"]);try{await invoke("start_translation",{payload:{quests_dir:scan.quests_dir,...settings}})}catch(e){setBusy(false);setStage("error");notify(String(e))}}
  async function saveSettings(){try{const r=await call<{credential_backend:string}>("save-settings",settings);setSettings(v=>({...v,has_api_key:!!v.api_key,credential_backend:r.credential_backend}));notify("设置已保存")}catch(e){notify(String(e))}}
  const warningCount=report?Object.keys(report.warnings).length:0;

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><QuestMark/><div><strong>FTB Translater</strong><span>任务书汉化工作台</span></div></div>
      <nav aria-label="主导航">
        <Nav active={view==="workbench"} icon={<BookOpen/>} label="翻译工作台" onClick={()=>setView("workbench")}/>
        <Nav active={view==="history"} icon={<History/>} label="翻译历史" onClick={()=>setView("history")} badge={runs.length||undefined}/>
        <Nav active={view==="settings"} icon={<Settings/>} label="服务设置" onClick={()=>setView("settings")}/>
      </nav>
      <div className="sidebar-note"><ShieldCheck/><div><strong>格式安全守卫</strong><span>自动保护颜色码、占位符与物品标签</span></div></div>
      <button className="theme-toggle" onClick={()=>setTheme(theme==="light"?"dark":"light")}>{theme==="light"?<Moon/>:<Sun/>}<span>{theme==="light"?"切换深色":"切换浅色"}</span></button>
    </aside>
    <main className="main-area">
      {view==="workbench"&&<Workbench stage={stage} scan={scan} path={selectedPath} setPath={setSelectedPath} busy={busy} progress={progress} logs={logs} report={report} warnings={warningCount} onChoose={chooseFolder} onScan={()=>doScan()} onTranslate={()=>setConfirm(true)} onSettings={()=>setView("settings")}/>} 
      {view==="settings"&&<SettingsPage value={settings} onChange={setSettings} onSave={saveSettings}/>} 
      {view==="history"&&<HistoryPage runs={runs} notify={notify} reload={loadHistory}/>} 
    </main>
    {confirm&&scan&&<ConfirmDialog scan={scan} onCancel={()=>setConfirm(false)} onConfirm={beginTranslation}/>} 
    {toast&&<div className="toast"><Check/>{toast}</div>}
  </div>
}

function Nav({active,icon,label,onClick,badge}:{active:boolean;icon:React.ReactNode;label:string;onClick:()=>void;badge?:number}){return <button className={`nav-item ${active?"active":""}`} onClick={onClick}>{icon}<span>{label}</span>{badge!==undefined&&<em>{badge}</em>}</button>}

function Workbench(p:{stage:Stage;scan:ScanResult|null;path:string;setPath:(v:string)=>void;busy:boolean;progress:number;logs:string[];report:Report|null;warnings:number;onChoose:()=>void;onScan:()=>void;onTranslate:()=>void;onSettings:()=>void}){
  const steps=[{key:"idle",label:"选择任务书"},{key:"scanned",label:"确认内容"},{key:"running",label:"自动汉化"},{key:"done",label:"检查结果"}]; const index={idle:0,scanned:1,running:2,done:3,error:p.scan?1:0}[p.stage];
  return <div className="page workbench-page">
    <header className="page-header"><div><p className="eyebrow">TRANSLATION WORKBENCH</p><h1>把任务书带给中文玩家</h1><p>选择整合包，确认扫描结果，然后安全写回汉化内容。</p></div><div className={`status-pill ${p.stage}`}><span/>{p.stage==="running"?"正在汉化":p.stage==="done"?"本次完成":p.stage==="error"?"需要处理":p.stage==="scanned"?"等待开始":"准备就绪"}</div></header>
    <section className="quest-chain" aria-label="汉化进度">{steps.map((s,i)=><React.Fragment key={s.key}><div className={`quest-step ${i<=index?"active":""} ${i<index?"complete":""}`}><i>{i<index?<Check/>:i+1}</i><span>{s.label}</span></div>{i<3&&<div className={`quest-link ${i<index?"active":""}`}><span/></div>}</React.Fragment>)}</section>
    <div className="workspace-grid">
      <section className="card source-card"><div className="card-title"><div className="icon-tile blue"><FolderOpen/></div><div><h2>任务书位置</h2><p>整合包根目录或 quests、lang、chapters 目录都可以</p></div></div><div className="path-control"><input value={p.path} onChange={e=>p.setPath(e.target.value)} placeholder="选择一个整合包目录…" onKeyDown={e=>e.key==="Enter"&&p.onScan()}/><button className="secondary" onClick={p.onChoose}><FolderOpen/>选择目录</button></div>{p.scan?<ScanSummary scan={p.scan}/>:<div className="drop-hint" onClick={p.onChoose}><FileSearch/><div><strong>从扫描开始</strong><span>我们会自动判断任务书格式，不会在扫描时改动文件。</span></div><ChevronRight/></div>}</section>
      <aside className="card action-card"><p className="eyebrow">NEXT ACTION</p>{p.stage==="idle"||p.stage==="error"&&!p.scan?<><h2>先找到任务书</h2><p>扫描只读取目录结构和可翻译条目，不会覆盖任何文件。</p><button className="primary wide" disabled={p.busy} onClick={p.onScan}>{p.busy?<RefreshCw className="spin"/>:<FileSearch/>}扫描任务书</button></>:p.stage==="scanned"?<><h2>准备开始汉化</h2><p>开始前会再次确认写入目标，并自动创建可恢复的完整备份。</p><button className="primary wide" onClick={p.onTranslate}><Play/>开始汉化<ArrowRight/></button><button className="text-button" onClick={p.onSettings}><Settings/>检查翻译设置</button></>:p.stage==="running"?<><h2>{p.progress<100?"正在翻译":"正在收尾"}</h2><p>可以保留此窗口在后台，完成后会自动保存历史。</p><div className="progress-number"><strong>{p.progress}</strong><span>%</span></div><div className="progress-track"><span style={{width:`${p.progress}%`}}/></div></>:<><h2>汉化已经写入</h2><p>{p.warnings?`有 ${p.warnings} 条内容建议人工确认，其余内容已完成。`:"格式检查全部通过，可以进入游戏查看效果。"}</p><div className="result-mini"><Check/><span>备份已创建</span></div><button className="secondary wide" onClick={p.onChoose}>翻译另一个整合包</button></>}</aside>
    </div>
    {(p.logs.length>0||p.report)&&<section className="lower-grid"><div className="card log-card"><div className="card-title compact"><div><h2>运行记录</h2><p>只显示对排查问题有帮助的信息</p></div><span className="live-dot">实时</span></div><div className="log-list">{p.logs.map((l,i)=><div key={i}><span>{String(i+1).padStart(2,"0")}</span><p>{l}</p></div>)}</div></div>{p.report&&<div className="card report-card"><div className="card-title compact"><div><h2>本次结果</h2><p>{p.report.translated_entries} / {p.report.total_entries} 条已处理</p></div></div><Metric label="缓存命中" value={p.report.cache_hits}/><Metric label="需要检查" value={p.warnings} warn={p.warnings>0}/><Metric label="翻译失败" value={p.report.failed_entries.length} warn={p.report.failed_entries.length>0}/><div className="backup-path"><Archive/><span title={p.report.backup_dir}>{p.report.backup_dir}</span><button onClick={()=>navigator.clipboard.writeText(p.report!.backup_dir)} aria-label="复制备份路径"><Copy/></button></div></div>}</section>}
    {p.report&&p.warnings>0&&<ReviewPanel report={p.report}/>} 
  </div>
}

function ScanSummary({scan}:{scan:ScanResult}){return <div className="scan-summary"><div className="pack-row"><div className="pack-icon"><QuestMark compact/></div><div><span>已识别整合包</span><strong>{scan.pack_name||"FTB Quests"}</strong></div><span className="mode-badge">{scan.mode_label}</span></div><div className="scan-stats"><div><strong>{scan.entry_count.toLocaleString()}</strong><span>可翻译条目</span></div><div><strong>{scan.file_count}</strong><span>{scan.mode==="lang"?"语言文件":"章节文件"}</span></div><div><strong>{scan.estimated_batches}</strong><span>预计请求批次</span></div></div><p className="mono-path">{scan.source}</p></div>}
function Metric({label,value,warn=false}:{label:string;value:number;warn?:boolean}){return <div className="metric"><span>{label}</span><strong className={warn?"warn":""}>{value}</strong></div>}

function ReviewPanel({report}:{report:Report}){const entries=Object.entries(report.warnings);return <section className="review-section"><div className="review-heading"><div><p className="eyebrow">MANUAL REVIEW</p><h2>检查格式告警</h2><p>守卫已经保留原文。确认颜色码、占位符和换行后，可直接修正写入。</p></div><span>{entries.length} 条待检查</span></div><div className="review-list">{entries.map(([key,warnings])=><ReviewCard key={key} entryKey={key} warnings={warnings} detail={report.failed_translations[key]} target={report.target_file}/>)}</div></section>}
function ReviewCard({entryKey,warnings,detail,target}:{entryKey:string;warnings:string[];detail?:{source:string;failed:string;error?:string};target:string}){const [text,setText]=useState(detail?.failed||detail?.source||"");const [status,setStatus]=useState("");async function saveText(){setStatus("正在保存…");try{await call("save-review",{target_file:target,key:entryKey,text});setStatus("已写入目标文件 ✓")}catch(e){setStatus(String(e))}}return <article className="review-card"><div className="review-key"><code>{entryKey}</code><span>{warnings.length} 个问题</span></div><div className="review-columns"><div><label>英文原文</label><p>{detail?.source||"未记录原文"}</p></div><div><label>修正后的中文</label><textarea value={text} onChange={e=>setText(e.target.value)} rows={3}/></div></div><ul>{warnings.map((w,i)=><li key={i}><CircleAlert/>{w}</li>)}</ul><div className="review-actions"><span>{status}</span><button className="secondary" onClick={saveText}><Save/>保存这条修正</button></div></article>}

function SettingsPage({value,onChange,onSave}:{value:SettingsData;onChange:(v:SettingsData)=>void;onSave:()=>void}){const [show,setShow]=useState(false);const update=(k:keyof SettingsData,v:string)=>onChange({...value,[k]:v});return <div className="page narrow-page"><header className="page-header"><div><p className="eyebrow">SERVICE SETTINGS</p><h1>翻译服务</h1><p>凭证留在本机，模型参数用于下一次翻译任务。</p></div></header><section className="settings-layout"><div className="card settings-card"><div className="section-heading"><KeyRound/><div><h2>DeepSeek 凭证</h2><p>API Key 通过系统安全凭证服务保存，不会写入项目文件。</p></div></div><label>API Key<div className="input-with-action"><input type={show?"text":"password"} value={value.api_key} onChange={e=>update("api_key",e.target.value)} placeholder="sk-…"/><button onClick={()=>setShow(!show)} aria-label={show?"隐藏密钥":"显示密钥"}>{show?<EyeOff/>:<Eye/>}</button></div></label><div className="security-note"><ShieldCheck/><span>当前存储位置：{value.credential_backend}</span></div></div><div className="card settings-card"><div className="section-heading"><Sparkles/><div><h2>模型与接口</h2><p>兼容 DeepSeek 的 OpenAI 风格接口。</p></div></div><div className="field-grid"><label>接口地址<input value={value.base_url} onChange={e=>update("base_url",e.target.value)}/></label><label>模型名称<input value={value.model} onChange={e=>update("model",e.target.value)}/></label><label>每批条目<input value={value.batch_size} onChange={e=>update("batch_size",e.target.value)} placeholder="auto"/><small>填写 auto 让程序按文本长度自动拆分</small></label><label>并发请求<input value={value.concurrency} onChange={e=>update("concurrency",e.target.value)} placeholder="auto"/><small>网络不稳定时可手动设为 2–4</small></label></div><label>翻译要求<textarea rows={5} value={value.style} onChange={e=>update("style",e.target.value)}/></label></div><div className="settings-actions"><button className="primary" onClick={onSave}><Save/>保存设置</button><span>修改将在下一次任务开始时生效</span></div></section></div>}

function HistoryPage({runs,notify,reload}:{runs:Run[];notify:(v:string)=>void;reload:()=>void}){const [query,setQuery]=useState("");const filtered=useMemo(()=>runs.filter(r=>`${r.pack_name} ${r.quests_dir}`.toLowerCase().includes(query.toLowerCase())),[runs,query]);async function remove(id:number){if(!window.confirm("删除这条历史记录？已经写入整合包的文件不会被删除。"))return;try{await call("history-delete",{run_id:id});reload();notify("历史记录已删除")}catch(e){notify(String(e))}}async function exportRun(r:Run){const target=await save({title:"导出汉化内容",defaultPath:`${r.pack_name||"ftb-translation"}-${r.id}.zip`,filters:[{name:"ZIP 压缩包",extensions:["zip"]}]});if(target)try{await call("history-export",{run_id:r.id,path:target});notify("ZIP 已导出")}catch(e){notify(String(e))}}return <div className="page"><header className="page-header history-header"><div><p className="eyebrow">TRANSLATION ARCHIVE</p><h1>翻译历史</h1><p>重新找到每一次写入、备份和可导出的汉化结果。</p></div><div className="search-box"><FileSearch/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索整合包或路径"/></div></header>{filtered.length?<div className="history-list">{filtered.map(r=><article className="history-row" key={r.id}><div className="history-date"><strong>{new Date(r.created_at).toLocaleDateString("zh-CN",{month:"short",day:"numeric"})}</strong><span>{new Date(r.created_at).toLocaleTimeString("zh-CN",{hour:"2-digit",minute:"2-digit"})}</span></div><div className="history-main"><div><h2>{r.pack_name||"未命名整合包"}</h2><span className="mode-badge">{r.mode==="lang"?"语言文件":"章节文件"}</span></div><p>{r.quests_dir}</p><div className="history-facts"><span><Check/>{r.translated_entries} 条完成</span><span className={r.warning_count?"warning":""}><CircleAlert/>{r.warning_count} 条检查</span><span>{r.model}</span></div></div><div className="history-actions"><button className="secondary" onClick={()=>exportRun(r)}><Archive/>导出</button><button className="icon-button danger" onClick={()=>remove(r.id)} aria-label="删除"><Trash2/></button></div></article>)}</div>:<div className="empty-state"><div><History/></div><h2>{query?"没有匹配的记录":"还没有翻译历史"}</h2><p>{query?"换一个整合包名称或目录关键词。":"完成第一次汉化后，结果会自动出现在这里。"}</p></div>}</div>}

function ConfirmDialog({scan,onCancel,onConfirm}:{scan:ScanResult;onCancel:()=>void;onConfirm:()=>void}){return <div className="modal-backdrop" onMouseDown={e=>e.target===e.currentTarget&&onCancel()}><div className="modal" role="dialog" aria-modal="true"><button className="modal-close" onClick={onCancel}><X/></button><div className="modal-icon"><Languages/></div><p className="eyebrow">READY TO TRANSLATE</p><h2>开始汉化 {scan.entry_count.toLocaleString()} 条内容？</h2><p>程序会先备份现有的 <code>{scan.mode==="lang"?"lang":"chapters"}</code> 目录，再覆盖写入译文。即使中途出现问题，也可以从备份恢复。</p><div className="confirm-target"><span>写入目标</span><strong>{scan.mode==="lang"?"lang/zh_cn.snbt":"chapters/*.snbt"}</strong></div><div className="modal-actions"><button className="secondary" onClick={onCancel}>暂不开始</button><button className="primary" onClick={onConfirm}><Play/>创建备份并汉化</button></div></div></div>}

createRoot(document.getElementById("root")!).render(<React.StrictMode><App/></React.StrictMode>);
