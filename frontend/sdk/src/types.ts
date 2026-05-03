// ─── Auto-generated backend types ───

import type {
  AdminCommentRow,
  ApiAccess,
  ApiConfig,
  ApiEndpointConfig,
  ApiTokenListItem,
  AuditEntry,
  AuthConfigResponse,
  AutoFillSource,
  Category,
  ColumnDef,
  CommentResponse,
  ContentKind,
  ContentRevision,
  ContentTypeSchema,
  CreateTokenResult,
  CronExecutionLog,
  CronSchedule,
  FaqItem,
  FieldSchema,
  FieldType,
  FormFieldDef,
  GalleryImage,
  IndexDef,
  ListViewConfig,
  LoginResponse,
  MediaConfig,
  MediaResponse,
  MediaStatsResponse,
  MediaTypeInfoResponse,
  OAuthBindingInfo,
  OptionEntry,
  OptionGroup,
  Page,
  PageBlock,
  Permission,
  Permissions,
  PluginEvent,
  PluginHealth,
  PluginInfoResponse,
  PluginMetrics,
  PostResponse,
  PricingPlan,
  ProviderInfo,
  RelationConfig,
  RelationType,
  ReusableBlock,
  RevisionSummary,
  Role,
  SitemapEntry,
  SocialLink,
  StatItem,
  StepDef,
  StepLog,
  StepType,
  Tag,
  TagBrief,
  TeamMember,
  Tenant,
  TestimonialItem,
  TimelineItem,
  UserResponse,
  WebhookSubscription,
  WorkflowDefinition,
  WorkflowInstance,
} from "./generated/types";

export type {
  AdminCommentRow,
  ApiAccess,
  ApiConfig,
  ApiEndpointConfig,
  ApiTokenListItem,
  AuditEntry,
  AuthConfigResponse,
  AutoFillSource,
  Category,
  ColumnDef,
  CommentResponse,
  ContentKind,
  ContentRevision,
  ContentTypeSchema,
  CreateTokenResult,
  CronExecutionLog,
  CronSchedule,
  FaqItem,
  FieldSchema,
  FieldType,
  FormFieldDef,
  GalleryImage,
  IndexDef,
  ListViewConfig,
  LoginResponse,
  MediaConfig,
  MediaResponse,
  MediaStatsResponse,
  MediaTypeInfoResponse,
  OAuthBindingInfo,
  OptionEntry,
  OptionGroup,
  Page,
  PageBlock,
  Permission,
  Permissions,
  PluginEvent,
  PluginHealth,
  PluginInfoResponse,
  PluginMetrics,
  PostResponse,
  PricingPlan,
  ProviderInfo,
  RelationConfig,
  RelationType,
  ReusableBlock,
  RevisionSummary,
  Role,
  SitemapEntry,
  SocialLink,
  StatItem,
  StepDef,
  StepLog,
  StepType,
  Tag,
  TagBrief,
  TeamMember,
  Tenant,
  TestimonialItem,
  TimelineItem,
  UserResponse,
  WebhookSubscription,
  WorkflowDefinition,
  WorkflowInstance,
};

// ─── Ergonomic aliases (with bigint→number fixes) ───

export type User = UserResponse;
export type AuthResult = Omit<LoginResponse, "expires_in"> & { expires_in: number };
export type MediaFile = Omit<MediaResponse, "size"> & { size: number };
export type MediaStats = Omit<MediaStatsResponse, "total_files" | "total_size"> & { total_files: number; total_size: number };
export type AuthConfig = AuthConfigResponse;
export type OAuthProvider = ProviderInfo;
export type OAuthBinding = OAuthBindingInfo;
export type PluginInfo = PluginInfoResponse;
export type FieldDef = FieldSchema;
export type Webhook = WebhookSubscription;
export type AuditLog = AuditEntry;
export type CronJob = CronSchedule;
export type CronLog = CronExecutionLog;
export type ApiToken = ApiTokenListItem;
export type Workflow = WorkflowDefinition;
export type Revision = RevisionSummary;

// ─── SDK-only types (not from backend) ───

export interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

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

export type BeforeSendHook = (
  url: string,
  options: RequestInit,
) => { url: string; options: RequestInit } | Promise<{ url: string; options: RequestInit }>;

export type AfterSendHook<T = unknown> = (
  response: Response,
  data: T,
) => T | Promise<T>;

export interface HealthStatus {
  status: string;
  version?: string;
  uptime?: number;
}

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

export interface RouteInfo {
  method: string;
  path: string;
  source: string;
  name: string;
}
