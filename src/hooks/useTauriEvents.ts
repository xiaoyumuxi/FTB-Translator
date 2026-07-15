import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import type { TranslationEvent } from "../models/translation";

export function useTauriEvents(onTranslationEvent: (event: TranslationEvent) => void) {
  const handler = useRef(onTranslationEvent);
  handler.current = onTranslationEvent;

  useEffect(() => {
    const unlisten = listen<TranslationEvent>("translation-event", ({ payload }) => {
      handler.current(payload);
    });

    return () => {
      void unlisten.then((removeListener) => removeListener());
    };
  }, []);
}
