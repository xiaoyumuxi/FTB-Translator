import React from "react";
import { createRoot } from "react-dom/client";
import { CmpTable } from "../../../src/components/CmpTable";
import type { CmpEntry } from "../../../src/models/cmp";
import "../../../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(window.location.search).get("theme") === "dark"
  ? "dark"
  : "light";

const entries: CmpEntry[] = [
  {
    index: 0,
    entry_id: "chapter-one:0:title",
    path: "$",
    file: "chapters/chapter-one.snbt",
    source: "Power",
    target: "能量",
    status: "translated",
  },
  {
    index: 1,
    entry_id: "chapter-two:0:title",
    path: "$",
    file: "chapters/chapter-two.snbt",
    source: "Power",
    target: "动力",
    status: "review",
  },
  {
    index: 2,
    entry_id: "chapter-two:1:description",
    path: "$",
    file: "chapters/chapter-two.snbt",
    source: "Welcome to the factory",
    target: "Welcome to the factory",
    status: "unchanged",
  },
  {
    index: 3,
    entry_id: "chapter-three:0:description",
    path: "$",
    file: "chapters/chapter-three.snbt",
    source: "Open the quest book",
    target: "Open quest menu",
    status: "translated",
  },
];

function Preview() {
  const [draftEntries, setDraftEntries] = React.useState(entries);
  return (
    <main className="page" style={{ maxWidth: 1180 }}>
      <CmpTable
        draft={{
          cmp_path: "/preview/review.cmp",
          total_entries: entries.length,
          warning_count: 3,
          failed_count: 0,
          can_apply: true,
        }}
        entries={draftEntries}
        setEntries={setDraftEntries}
        validation={null}
        validating={false}
        onOpen={() => {}}
        onExport={() => {}}
        onChoose={() => {}}
        onValidate={() => {}}
        onApply={() => {}}
      />
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<Preview />);
