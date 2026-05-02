// ─── Core ───

export interface User {
  id: string;
  email: string;
  nickname: string;
  role: string;
  avatar: string | null;
  tenant_id: string;
}

export interface AuthResult {
  access_token: string;
  refresh_token: string;
  user: User;
}

export interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

// ─── Request Options ───

export interface RequestOptions {
  headers?: Record<string, string>;
  query?: Record<string, string>;
  signal?: AbortSignal;
  fetch?: typeof fetch;
  requestKey?: string;
}

export interface ListOptions extends RequestOptions {
  sort?: string;
  filter?: string;
  search?: string;
  fields?: string;
  status?: string;
  expand?: string;
}

export interface MutateOptions extends RequestOptions {
  expand?: string;
}

export interface SendOptions extends RequestOptions {
  method?: string;
  body?: unknown;
}

// ─── Auth Store ───

export type AuthStoreListener = (
  token: string | null,
  user: User | null,
) => void;

export interface IAuthStore {
  readonly token: string | null;
  readonly refreshToken: string | null;
  readonly user: User | null;
  readonly isAuthenticated: boolean;
  save(auth: AuthResult): void;
  clear(): void;
  onChange(callback: AuthStoreListener, fireImmediately?: boolean): () => void;
}

// ─── Send Hooks ───

export type BeforeSendHook = (
  url: string,
  options: RequestInit,
) => { url: string; options: RequestInit } | Promise<{ url: string; options: RequestInit }>;

export type AfterSendHook<T = unknown> = (
  response: Response,
  data: T,
) => T | Promise<T>;

// ─── Auth Config ───

export interface AuthConfig {
  oauth_providers: string[];
  sms_enabled: boolean;
  email_verification_enabled: boolean;
  registration_enabled: boolean;
}

// ─── OAuth ───

export interface OAuthProvider {
  name: string;
  auth_url: string;
}

export interface OAuthBinding {
  provider: string;
  provider_user_id: string;
  linked_at: string;
}

// ─── Media ───

export interface MediaFile {
  id: string;
  user_id: string;
  filename: string;
  url: string;
  mimetype: string;
  size: number;
  width: number | null;
  height: number | null;
  created_at: string;
}

export interface MediaStats {
  total_files: number;
  total_size: number;
  by_type: Record<string, number>;
}

// ─── Health ───

export interface HealthStatus {
  status: string;
  version?: string;
  uptime?: number;
}

// ─── Revision ───

export interface Revision {
  id: string;
  record_id: string;
  version: number;
  data: Record<string, unknown>;
  created_by: string;
  created_at: string;
}

// ─── Admin Stats ───

export interface AdminStats {
  posts: number;
  pages: number;
  comments: number;
  categories: number;
  tags: number;
  media: number;
  users: number;
}

export interface TrendPoint {
  date: string;
  count: number;
}

// ─── Plugin ───

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  description: string;
  runtime: string;
  enabled: boolean;
}

// ─── Content Type ───

export interface ContentTypeSchema {
  name: string;
  singular: string;
  plural: string;
  table: string;
  kind: string;
  fields: Record<string, FieldDef>;
  implements: string[];
}

export interface FieldDef {
  type: string;
  required?: boolean;
  label?: string;
  default?: unknown;
  options?: string[];
}

// ─── Route ───

export interface RouteInfo {
  method: string;
  path: string;
  source: string;
  name: string;
}

// ─── Tenant ───

export interface Tenant {
  id: string;
  name: string;
  slug: string;
  status: string;
  created_at: string;
  updated_at: string;
}

// ─── RBAC ───

export interface Role {
  id: string;
  name: string;
  description: string | null;
  created_at: string;
}

export interface Permission {
  resource: string;
  action: string;
  conditions?: Record<string, unknown>;
}

// ─── Options ───

export interface Option {
  key: string;
  value: string;
  created_at: string;
  updated_at: string;
}

// ─── Webhook ───

export interface Webhook {
  id: string;
  url: string;
  events: string[];
  secret: string | null;
  enabled: boolean;
  last_triggered_at: string | null;
  created_at: string;
  updated_at: string;
}

// ─── Audit ───

export interface AuditLog {
  id: string;
  action: string;
  actor_id: string;
  resource_type: string;
  resource_id: string;
  details: Record<string, unknown> | null;
  ip: string | null;
  created_at: string;
}

// ─── Cron ───

export interface CronJob {
  id: string;
  name: string;
  schedule: string;
  handler: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface CronLog {
  id: string;
  cron_id: string;
  status: string;
  started_at: string;
  finished_at: string | null;
  error: string | null;
}

// ─── API Token ───

export interface ApiToken {
  id: string;
  name: string;
  token_preview: string;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

// ─── Workflow ───

export interface Workflow {
  id: string;
  name: string;
  description: string | null;
  steps: WorkflowStep[];
  created_at: string;
  updated_at: string;
}

export interface WorkflowStep {
  id: string;
  name: string;
  type: string;
  config: Record<string, unknown>;
  next?: string;
}

export interface WorkflowInstance {
  id: string;
  workflow_id: string;
  status: string;
  current_step: string;
  context: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface StepLog {
  id: string;
  instance_id: string;
  step_id: string;
  status: string;
  actor_id: string | null;
  data: Record<string, unknown> | null;
  created_at: string;
}
