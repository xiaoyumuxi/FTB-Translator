# Golden fixture data

These fixtures exercise the production scan, extraction, CMP serialization, format guard,
backup, and writeback paths without using a translation service or credential store.
`mock-translations.json` is a deterministic provider response keyed by CMP entry ID and
JSON pointer.

Each end-to-end case contains:

- `input/`: a complete minimal FTB Quests directory;
- `expected-extraction.json`: ordered entries and translation units;
- `mock-translations.json`: deterministic offline translations;
- `expected.cmp`: serialized CMP with `{{QUESTS_DIR}}` in place of the temporary path;
- `expected/`: byte-for-byte expected writeback files.

Coverage:

- `lang-rich`: language mode, multiline arrays, JSON rich text, format/colour codes,
  selectors, resource IDs, quotes, backslashes, newlines, duplicate JSON keys, and
  syntactically malformed JSON components. Unsafe JSON is deliberately retained.
- `chapters-nested`: chapter mode, nested SNBT, multiple files, single and double quotes,
  rich text, colour codes, selectors, numbers, resource IDs, arrows, and escaping.
- `multi-file-rollback`: deterministic commit failure after the first file is written;
  the golden file proves the first write is restored.

`invalid_late_record_in_multi_file_cmp_writes_nothing` additionally corrupts a record in
the second chapter file and verifies that validation rejects the whole CMP before backup
or writeback.
