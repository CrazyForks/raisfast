import { HttpClient, type ApiStyle } from "./client";
import { LocalAuthStore } from "./auth";
import { Auth } from "./public/auth";
import { Users } from "./public/users";
import { Posts } from "./public/posts";
import { Pages } from "./public/pages";
import { Comments } from "./public/comments";
import { Categories } from "./public/categories";
import { Tags } from "./public/tags";
import { Media } from "./public/media";
import { Health } from "./public/health";
import { Wallets } from "./public/wallets";
import { Events } from "./public/events";
import { Orders } from "./public/orders";
import { Payment } from "./public/payment";
import { Products } from "./public/products";
import { Admin } from "./admin";
import { Collection } from "./collection";
import { SDKError } from "./errors";
import type {
  AfterSendHook,
  BeforeSendHook,
  IAuthStore,
  ListOptions,
  SendOptions,
} from "./types";

interface ServerInfo {
  name: string;
  version: string;
  api_style: ApiStyle;
}

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
  readonly wallets: Wallets;
  readonly events: Events;
  readonly orders: Orders;
  readonly payment: Payment;
  readonly products: Products;
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
    this.wallets = new Wallets(this.http);
    this.events = new Events(this.http.baseUrl);
    this.orders = new Orders(this.http);
    this.payment = new Payment(this.http);
    this.products = new Products(this.http);
  }

  async init(): Promise<void> {
    try {
      const res = await fetch(`${this.http.baseUrl}/info`);
      const json = (await res.json()) as { code: number; data: ServerInfo };
      if (json.code === 0 && json.data?.api_style) {
        this.http.apiStyle = json.data.api_style;
      }
    } catch {
      // keep default restful
    }
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

  getRssFeedURL(): string {
    const base = this.http.baseUrl.replace("/api/v1", "");
    return `${base}/feed.xml`;
  }
}
