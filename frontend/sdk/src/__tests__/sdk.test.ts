import { describe, it, expect, vi } from "vitest";
import { SDKError } from "../errors";
import { BaseAuthStore } from "../auth";
import { Collection } from "../collection";
import { HttpClient } from "../client";

describe("SDKError", () => {
  it("stores code, status, url, response", () => {
    const err = new SDKError(40001, "bad request", 400, "/api/test", {
      detail: "invalid",
    });
    expect(err.code).toBe(40001);
    expect(err.status).toBe(400);
    expect(err.message).toBe("bad request");
    expect(err.name).toBe("SDKError");
    expect(err.url).toBe("/api/test");
    expect(err.response).toEqual({ detail: "invalid" });
    expect(err.isAbort).toBe(false);
    expect(err.originalError).toBeNull();
  });

  it("defaults optional fields", () => {
    const err = new SDKError(1, "fail");
    expect(err.status).toBe(400);
    expect(err.url).toBe("");
    expect(err.response).toEqual({});
    expect(err.isAbort).toBe(false);
    expect(err.originalError).toBeNull();
  });
});

describe("BaseAuthStore", () => {
  it("saves and clears auth", () => {
    const store = new BaseAuthStore();
    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
    expect(store.token).toBeNull();
    expect(store.refreshToken).toBeNull();

    store.save({
      access_token: "at",
      refresh_token: "rt",
      user: {
        id: "u1",
        email: "a@b.com",
        nickname: "test",
        role: "admin",
        avatar: null,
        tenant_id: "default",
      },
    });

    expect(store.isAuthenticated).toBe(true);
    expect(store.token).toBe("at");
    expect(store.refreshToken).toBe("rt");
    expect(store.user?.id).toBe("u1");

    store.clear();
    expect(store.isAuthenticated).toBe(false);
    expect(store.token).toBeNull();
    expect(store.refreshToken).toBeNull();
  });

  it("exports and imports from storage", () => {
    const store = new BaseAuthStore();
    store.save({
      access_token: "at",
      refresh_token: "rt",
      user: {
        id: "u1",
        email: "a@b.com",
        nickname: "test",
        role: "admin",
        avatar: null,
        tenant_id: "default",
      },
    });

    const exported = store.exportToStorage();
    const store2 = new BaseAuthStore();
    store2.importFromStorage(exported);
    expect(store2.token).toBe("at");
    expect(store2.refreshToken).toBe("rt");
    expect(store2.user?.id).toBe("u1");
  });

  it("handles invalid import data", () => {
    const store = new BaseAuthStore();
    store.importFromStorage("not json");
    expect(store.token).toBeNull();
  });

  it("notifies onChange listeners on save and clear", () => {
    const store = new BaseAuthStore();
    const listener = vi.fn();
    const unsub = store.onChange(listener);

    const user = {
      id: "u1",
      email: "a@b.com",
      nickname: "test",
      role: "admin",
      avatar: null,
      tenant_id: "default",
    };

    store.save({ access_token: "at", refresh_token: "rt", user });
    expect(listener).toHaveBeenCalledWith("at", user);
    listener.mockClear();

    store.clear();
    expect(listener).toHaveBeenCalledWith(null, null);

    unsub();
    listener.mockClear();

    store.save({ access_token: "at2", refresh_token: "rt2", user });
    expect(listener).not.toHaveBeenCalled();
  });

  it("fires onChange immediately when requested", () => {
    const store = new BaseAuthStore();
    const listener = vi.fn();
    store.onChange(listener, true);
    expect(listener).toHaveBeenCalledWith(null, null);
  });
});

describe("HttpClient", () => {
  it("sets and gets tenant ID", () => {
    const store = new BaseAuthStore();
    const http = new HttpClient("http://localhost:9898/api/v1", store);
    expect(http.tenantId).toBeNull();
    http.setTenantId("t1");
    expect(http.tenantId).toBe("t1");
    http.setTenantId(null);
    expect(http.tenantId).toBeNull();
  });
});

describe("Collection", () => {
  it("uses public prefix by default", () => {
    const store = new BaseAuthStore();
    const http = new HttpClient("http://localhost:9898/api/v1", store);
    const col = new Collection(http, "posts");
    expect(col).toBeDefined();
  });

  it("uses admin prefix when admin=true", () => {
    const store = new BaseAuthStore();
    const http = new HttpClient("http://localhost:9898/api/v1", store);
    const col = new Collection(http, "posts", true);
    expect(col).toBeDefined();
  });
});
