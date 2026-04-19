"use client";

import { create } from "zustand";
import { persist } from "zustand/middleware";
import en from "./locales/en";
import zh from "./locales/zh";

export type Locale = "en" | "zh";

type TranslationMap = Record<string, string>;

const localeMap: Record<Locale, TranslationMap> = { en, zh };

interface I18nState {
  locale: Locale;
  setLocale: (locale: Locale) => void;
}

export const useI18nStore = create<I18nState>()(
  persist(
    (set) => ({
      locale: "en" as Locale,
      setLocale: (locale) => set({ locale }),
    }),
    { name: "i18n-locale" },
  ),
);

export function useT() {
  const locale = useI18nStore((s) => s.locale);
  const dict = localeMap[locale];

  function t(key: string, params?: Record<string, string | number>): string {
    let val = dict[key] ?? localeMap.en[key] ?? key;
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        val = val.replace(`{${k}}`, String(v));
      }
    }
    return val;
  }

  return { t, locale };
}
