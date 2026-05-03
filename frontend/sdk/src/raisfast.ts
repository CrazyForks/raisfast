import { HttpClient } from "./client";
import { LocalAuthStore } from "./auth";
import { Auth } from "./auth-api";
import { Admin } from "./admin";
import { Collection } from "./collection";
import { Users } from "./users";
import { Media } from "./media";
import { Health } from "./health";
import { Posts } from "./posts";
import { Categories } from "./categories";
import { Tags } from "./tags";
import { Comments } from "./comments";
import { Pages } from "./pages";
import { SDKError } from "./errors";
import type {
  AfterSendHook,
  BeforeSendHook,
  IAuthStore,
  ListOptions,
  SendOptions,
} from "./types";

export class RaisFast {
  readonly auth: Auth;
  readonly admin: Admin;
  readonly users: Users;
  readonly media: Media;
  readonly health: Health;
  readonly posts: Posts;
  readonly categories: Categories;
  readonly tags: Tags;
  readonly comments: Comments;
  readonly pages: Pages;
  readonly authStore: IAuthStore;
  private readonly http: HttpClient;

  constructor(
    baseUrl: string,
    options?: { authStore?: IAuthStore },
  ) {
    this.authStore = options?.authStore ?? new LocalAuthStore();
    this.http = new HttpClient(baseUrl, this.authStore);
    this.auth = new Auth(this.http, this.authStore);
    this.admin = new Admin(this.http);
    this.users = new Users(this.http);
    this.media = new Media(this.http);
    this.health = new Health(this.http);
    this.posts = new Posts(this.http);
    this.categories = new Categories(this.http);
    this.tags = new Tags(this.http);
    this.comments = new Comments(this.http);
    this.pages = new Pages(this.http);
  }

  collection<T = Record<string, unknown>>(name: string): Collection<T> {
    return new Collection<T>(this.http, name);
  }

  adminCollection<T = Record<string, unknown>>(name: string): Collection<T> {
    return new Collection<T>(this.http, name, true);
  }

  setTenantId(tenantId: string | null): void {
    this.http.setTenantId(tenantId);
  }

  set beforeSend(hook: BeforeSendHook | null) {
    this.http.beforeSend = hook;
  }

  set afterSend(hook: AfterSendHook | null) {
    this.http.afterSend = hook;
  }

  cancelRequest(key: string): void {
    this.http.cancelRequest(key);
  }

  cancelAllRequests(): void {
    this.http.cancelAllRequests();
  }

  async send<T>(path: string, options?: SendOptions): Promise<T> {
    return this.http.request<T>(path, options);
  }

  async single<T>(
    collectionName: string,
    options?: ListOptions,
  ): Promise<T> {
    const col = this.collection<T>(collectionName);
    const result = await col.getList(1, 1, options);
    if (result.items.length === 0) {
      throw new SDKError(
        404,
        `No ${collectionName} item found`,
        404,
      );
    }
    return result.items[0];
  }
}
