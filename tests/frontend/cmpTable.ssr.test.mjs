import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

test("renders CMP review clues with the editable table", async () => {
  const vite = await createServer({
    appType: "custom",
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  try {
    const { CmpTable } = await vite.ssrLoadModule("/src/components/CmpTable.tsx");
    const entries = [
      {
        index: 0,
        entry_id: "power-a",
        path: "$",
        file: "lang/en_us.snbt",
        source: "Power",
        target: "能量",
        status: "translated",
      },
      {
        index: 1,
        entry_id: "power-b",
        path: "$",
        file: "lang/en_us.snbt",
        source: "Power",
        target: "动力",
        status: "review",
      },
    ];
    const html = renderToStaticMarkup(
      React.createElement(CmpTable, {
        draft: {
          cmp_path: "/tmp/review.cmp",
          total_entries: entries.length,
          warning_count: 1,
          failed_count: 0,
          can_apply: true,
        },
        entries,
        setEntries: () => {},
        validation: null,
        validating: false,
        onOpen: () => {},
        onExport: () => {},
        onChoose: () => {},
        onValidate: () => {},
        onApply: () => {},
      }),
    );

    assert.match(html, /审校线索/);
    assert.match(html, /同源多译/);
    assert.match(html, />2<\/strong>/);
    assert.match(html, /能量/);
    assert.match(html, /动力/);
  } finally {
    await vite.close();
  }
});
