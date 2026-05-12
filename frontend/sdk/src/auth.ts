import type { AuthResult, AuthStoreListener, IAuthStore, User } from "./types";

export class BaseAuthStore implements IAuthStore {
  protected _token: string | null = null;
  protected _refreshToken: string | null = null;
  protected _user: User | null = null;
  private _listeners = new Set<AuthStoreListener>();

  get token(): string | null {
    return this._token;
  }

  get refreshToken(): string | null {
    return this._refreshToken;
  }

  get user(): User | null {
    return this._user;
  }

  get isAuthenticated(): boolean {
    return !!this._token;
  }

  save(auth: AuthResult): void {
    this._token = auth.access_token;
    this._refreshToken = auth.refresh_token;
    this._user = auth.user;
    this._notify();
  }

  clear(): void {
    this._token = null;
    this._refreshToken = null;
    this._user = null;
    this._notify();
  }

  onChange(
    callback: AuthStoreListener,
    fireImmediately = false,
  ): () => void {
    this._listeners.add(callback);
    if (fireImmediately) {
      callback(this._token, this._user);
    }
    return () => {
      this._listeners.delete(callback);
    };
  }

  exportToStorage(): string {
    return JSON.stringify({
      token: this._token,
      refreshToken: this._refreshToken,
      user: this._user,
    });
  }

  importFromStorage(data: string): void {
    try {
      const parsed = JSON.parse(data);
      this._token = parsed.token ?? null;
      this._refreshToken = parsed.refreshToken ?? null;
      this._user = parsed.user ?? null;
    } catch {
      // silent — don't trigger onChange on invalid data
    }
  }

  protected _notify(): void {
    for (const cb of this._listeners) {
      cb(this._token, this._user);
    }
  }
}

export class LocalAuthStore extends BaseAuthStore {
  private readonly _storageKey: string;
  private readonly _storage: Storage | null;

  constructor(storageKey = "raisfast_auth", storage?: Storage) {
    super();
    this._storageKey = storageKey;
    this._storage =
      storage ?? (typeof window !== "undefined" ? window.localStorage : null);
    this._load();
  }

  save(auth: AuthResult): void {
    super.save(auth);
    this._persist();
  }

  clear(): void {
    super.clear();
    this._persist();
  }

  private _persist(): void {
    try {
      this._storage?.setItem(this._storageKey, this.exportToStorage());
    } catch {
      // ignore storage errors
    }
  }

  private _load(): void {
    try {
      const data = this._storage?.getItem(this._storageKey);
      if (data) this.importFromStorage(data);
    } catch {
      // ignore storage errors
    }
  }
}
