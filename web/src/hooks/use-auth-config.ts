"use client";

import { useEffect, useState, useRef, useCallback } from "react";
import { api } from "@/lib/api";

export interface AuthConfig {
  registration_email_enabled: boolean;
  registration_sms_enabled: boolean;
  oauth_providers: string[];
  require_email_verification: boolean;
}

let cachedConfig: AuthConfig | null = null;
let fetchPromise: Promise<AuthConfig> | null = null;

async function fetchAuthConfig(): Promise<AuthConfig> {
  if (cachedConfig) return cachedConfig;
  if (fetchPromise) return fetchPromise;
  fetchPromise = api.get<AuthConfig>("/auth/config").then((config) => {
    cachedConfig = config;
    return config;
  });
  return fetchPromise;
}

export function useAuthConfig() {
  const [config, setConfig] = useState<AuthConfig>({
    registration_email_enabled: true,
    registration_sms_enabled: false,
    oauth_providers: [],
    require_email_verification: false,
  });
  const [loading, setLoading] = useState(true);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;
    fetchAuthConfig()
      .then(setConfig)
      .finally(() => setLoading(false));
  }, []);

  return { config, loading };
}

export function useSmsCountdown() {
  const [countdown, setCountdown] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const start = useCallback(() => {
    setCountdown(60);
    timerRef.current = setInterval(() => {
      setCountdown((prev) => {
        if (prev <= 1) {
          if (timerRef.current) clearInterval(timerRef.current);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);
  }, []);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, []);

  return { countdown, start };
}
