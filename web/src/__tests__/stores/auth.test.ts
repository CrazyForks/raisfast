import { describe, it, expect, beforeEach } from "vitest";
import { useAuthStore } from "@/stores/auth";

describe("useAuthStore", () => {
  beforeEach(() => {
    useAuthStore.getState().logout();
  });

  it("starts logged out", () => {
    const state = useAuthStore.getState();
    expect(state.isLoggedIn()).toBe(false);
    expect(state.user).toBeNull();
    expect(state.accessToken).toBeNull();
  });

  it("login sets user and tokens", () => {
    const store = useAuthStore.getState();
    store.login(
      { id: "1", email: "a@b.com", username: "alice", role: "admin", avatar: null, bio: null },
      "access-token-123",
      "refresh-token-456",
    );

    const state = useAuthStore.getState();
    expect(state.isLoggedIn()).toBe(true);
    expect(state.user?.username).toBe("alice");
    expect(state.accessToken).toBe("access-token-123");
  });

  it("logout clears everything", () => {
    const store = useAuthStore.getState();
    store.login(
      { id: "1", email: "a@b.com", username: "alice", role: "admin", avatar: null, bio: null },
      "access",
      "refresh",
    );
    store.logout();

    const state = useAuthStore.getState();
    expect(state.isLoggedIn()).toBe(false);
    expect(state.user).toBeNull();
  });

  it("isAdmin checks role", () => {
    const store = useAuthStore.getState();
    store.login(
      { id: "1", email: "a@b.com", username: "alice", role: "admin", avatar: null, bio: null },
      "token",
      "refresh",
    );
    expect(store.isAdmin()).toBe(true);
  });

  it("isAuthor returns true for admin", () => {
    const store = useAuthStore.getState();
    store.login(
      { id: "1", email: "a@b.com", username: "alice", role: "admin", avatar: null, bio: null },
      "token",
      "refresh",
    );
    expect(store.isAuthor()).toBe(true);
  });

  it("isAuthor returns true for author role", () => {
    const store = useAuthStore.getState();
    store.login(
      { id: "2", email: "b@b.com", username: "bob", role: "author", avatar: null, bio: null },
      "token",
      "refresh",
    );
    expect(store.isAuthor()).toBe(true);
    expect(store.isAdmin()).toBe(false);
  });

  it("isAuthor returns false for reader", () => {
    const store = useAuthStore.getState();
    store.login(
      { id: "3", email: "c@b.com", username: "carol", role: "reader", avatar: null, bio: null },
      "token",
      "refresh",
    );
    expect(store.isAuthor()).toBe(false);
    expect(store.isAdmin()).toBe(false);
  });
});
