import { HttpClient } from "../client";
import type { RequestOptions, RouteInfo } from "../types";
import { AdminAudit } from "./audit";
import { AdminCategories } from "./categories";
import { AdminComments } from "./comments";
import { AdminContentTypes } from "./content-types";
import { AdminCrons } from "./crons";
import { AdminCurrencies } from "./currencies";
import { AdminMedia } from "./media";
import { AdminOptions } from "./options";
import { AdminOrders } from "./orders";
import { AdminPages } from "./pages";
import { AdminPayment } from "./payment";
import { AdminPlugins } from "./plugins";
import { AdminPosts } from "./posts";
import { AdminProducts } from "./products";
import { AdminRBAC } from "./rbac";
import { AdminReusableBlocks } from "./reusable-blocks";
import { AdminStats } from "./stats";
import { AdminTags } from "./tags";
import { AdminTenants } from "./tenants";
import { AdminTokens } from "./tokens";
import { AdminUsers } from "./users";
import { AdminWallets } from "./wallets";
import { AdminWebhooks } from "./webhooks";
import { AdminWorkflows } from "./workflows";

export class Admin {
  readonly users: AdminUsers;
  readonly posts: AdminPosts;
  readonly pages: AdminPages;
  readonly comments: AdminComments;
  readonly categories: AdminCategories;
  readonly tags: AdminTags;
  readonly media: AdminMedia;
  readonly reusableBlocks: AdminReusableBlocks;
  readonly wallets: AdminWallets;
  readonly currencies: AdminCurrencies;
  readonly plugins: AdminPlugins;
  readonly contentTypes: AdminContentTypes;
  readonly tenants: AdminTenants;
  readonly rbac: AdminRBAC;
  readonly options: AdminOptions;
  readonly webhooks: AdminWebhooks;
  readonly audit: AdminAudit;
  readonly crons: AdminCrons;
  readonly tokens: AdminTokens;
  readonly workflows: AdminWorkflows;
  readonly stats: AdminStats;
  readonly orders: AdminOrders;
  readonly payment: AdminPayment;
  readonly products: AdminProducts;

  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
    this.users = new AdminUsers(http);
    this.posts = new AdminPosts(http);
    this.pages = new AdminPages(http);
    this.comments = new AdminComments(http);
    this.categories = new AdminCategories(http);
    this.tags = new AdminTags(http);
    this.media = new AdminMedia(http);
    this.reusableBlocks = new AdminReusableBlocks(http);
    this.wallets = new AdminWallets(http);
    this.currencies = new AdminCurrencies(http);
    this.plugins = new AdminPlugins(http);
    this.contentTypes = new AdminContentTypes(http);
    this.tenants = new AdminTenants(http);
    this.rbac = new AdminRBAC(http);
    this.options = new AdminOptions(http);
    this.webhooks = new AdminWebhooks(http);
    this.audit = new AdminAudit(http);
    this.crons = new AdminCrons(http);
    this.tokens = new AdminTokens(http);
    this.workflows = new AdminWorkflows(http);
    this.stats = new AdminStats(http);
    this.orders = new AdminOrders(http);
    this.payment = new AdminPayment(http);
    this.products = new AdminProducts(http);
  }

  async listRoutes(options?: RequestOptions): Promise<RouteInfo[]> {
    return this.http.get<RouteInfo[]>("/routes", options);
  }
}
