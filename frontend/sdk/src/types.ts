// ─── Auto-generated backend types ───

import type {
  AdminCommentListQuery,
  AdminCommentRow,
  AdminPageListQuery,
  AdminPostListQuery,
  AdminWalletOperationRequest,
  ApiAccess,
  ApiConfig,
  ApiEndpointConfig,
  ApiTokenListItem,
  AuditEntry,
  AuthConfigResponse,
  AuthType,
  BatchRequest,
  BatchRequestWithRole,
  BatchResponse,
  BindEmailRequest,
  BindPhoneRequest,
  Category,
  ColumnDef,
  CommentOpenStatus,
  CommentResponse,
  CommentStatus,
  ContentKind,
  ContentRevision,
  ContentTypeSchema,
  CreateCategoryRequest,
  CreateCommentRequest,
  CreateCurrencyRequest,
  CreatePageRequest,
  CreatePostRequest,
  CreateTagRequest,
  CreateTokenResult,
  CreateWorkflowRequest,
  CredentialResponse,
  CronExecStatus,
  CronExecutionLog,
  CronSchedule,
  CurrencyResponse,
  ExecuteStepRequest,
  FaqItem,
  FieldSchema,
  FieldType,
  ForgotPasswordRequest,
  FormFieldDef,
  GalleryImage,
  IndexDef,
  InstanceQuery,
  JobStatus,
  LoginRequest,
  LoginResponse,
  MediaConfig,
  MediaResponse,
  MediaStatsResponse,
  MediaTypeInfoResponse,
  OAuthBindingInfo,
  OptionEntry,
  OptionGroup,
  OptionType,
  Page,
  PageBlock,
  PageListQuery,
  PageStatus,
  Permission,
  Permissions,
  PluginEvent,
  PluginHealth,
  PluginInfoResponse,
  PluginMetrics,
  PostListQuery,
  PostResponse,
  PostStatus,
  PricingPlan,
  ProviderInfo,
  RefreshRequest,
  RegisterRequest,
  RegisteredVia,
  RelationConfig,
  RelationType,
  ReorderItem,
  ReorderRequest,
  ResendVerificationRequest,
  ResetPasswordRequest,
  ReusableBlock,
  ReversalRequest,
  RevisionSummary,
  Role,
  SendSmsCodeRequest,
  SetPasswordRequest,
  SitemapEntry,
  SocialLink,
  StartWorkflowRequest,
  StatItem,
  StepDef,
  StepLog,
  StepType,
  Tag,
  TagBrief,
  TeamMember,
  Tenant,
  TenantStatus,
  TestimonialItem,
  TimelineItem,
  UpdateCategoryRequest,
  UpdateCommentStatusRequest,
  UpdateCurrencyRequest,
  UpdatePageRequest,
  UpdatePasswordRequest,
  UpdatePostRequest,
  UpdateRoleRequest,
  UpdateStatusRequest,
  UpdateTagRequest,
  UpdateUserRequest,
  UserResponse,
  UserRole,
  UserStatus,
  VerifyEmailRequest,
  VerifySmsRequest,
  WalletEntryType,
  WalletReferenceType,
  WalletResponse,
  WalletStatus,
  WalletTransactionResponse,
  WalletTxType,
  WebhookSubscription,
  WorkflowDefinition,
  WorkflowInstance,
  WorkflowInstanceStatus,
  WorkflowStepStatus,
  ProductResponse,
  OrderItemResponse,
  OrderResponse,
  OrderStatsResponse,
  PaymentStatus,
  PaymentOrderResponse,
  PaymentChannelResponse,
  PaymentTransactionResponse,
  PaymentRefundResponse,
  AvailableChannelItem,
  AvailableChannelsResponse,
} from "./generated/types";

export type {
  AdminCommentListQuery,
  AdminCommentRow,
  AdminPageListQuery,
  AdminPostListQuery,
  AdminWalletOperationRequest,
  ApiAccess,
  ApiConfig,
  ApiEndpointConfig,
  ApiTokenListItem,
  AuditEntry,
  AuthConfigResponse,
  AuthType,
  BatchRequest,
  BatchRequestWithRole,
  BatchResponse,
  BindEmailRequest,
  BindPhoneRequest,
  Category,
  ColumnDef,
  CommentOpenStatus,
  CommentResponse,
  CommentStatus,
  ContentKind,
  ContentRevision,
  ContentTypeSchema,
  CreateCategoryRequest,
  CreateCommentRequest,
  CreateCurrencyRequest,
  CreatePageRequest,
  CreatePostRequest,
  CreateTagRequest,
  CreateTokenResult,
  CreateWorkflowRequest,
  CredentialResponse,
  CronExecStatus,
  CronExecutionLog,
  CronSchedule,
  CurrencyResponse,
  ExecuteStepRequest,
  FaqItem,
  FieldSchema,
  FieldType,
  ForgotPasswordRequest,
  FormFieldDef,
  GalleryImage,
  IndexDef,
  InstanceQuery,
  JobStatus,
  LoginRequest,
  LoginResponse,
  MediaConfig,
  MediaResponse,
  MediaStatsResponse,
  MediaTypeInfoResponse,
  OAuthBindingInfo,
  OptionEntry,
  OptionGroup,
  OptionType,
  Page,
  PageBlock,
  PageListQuery,
  PageStatus,
  Permission,
  Permissions,
  PluginEvent,
  PluginHealth,
  PluginInfoResponse,
  PluginMetrics,
  PostListQuery,
  PostResponse,
  PostStatus,
  PricingPlan,
  ProviderInfo,
  RefreshRequest,
  RegisterRequest,
  RegisteredVia,
  RelationConfig,
  RelationType,
  ReorderItem,
  ReorderRequest,
  ResendVerificationRequest,
  ResetPasswordRequest,
  ReusableBlock,
  ReversalRequest,
  RevisionSummary,
  Role,
  SendSmsCodeRequest,
  SetPasswordRequest,
  SitemapEntry,
  SocialLink,
  StartWorkflowRequest,
  StatItem,
  StepDef,
  StepLog,
  StepType,
  Tag,
  TagBrief,
  TeamMember,
  Tenant,
  TenantStatus,
  TestimonialItem,
  TimelineItem,
  UpdateCategoryRequest,
  UpdateCommentStatusRequest,
  UpdateCurrencyRequest,
  UpdatePageRequest,
  UpdatePasswordRequest,
  UpdatePostRequest,
  UpdateRoleRequest,
  UpdateStatusRequest,
  UpdateTagRequest,
  UpdateUserRequest,
  UserResponse,
  UserRole,
  UserStatus,
  VerifyEmailRequest,
  VerifySmsRequest,
  WalletEntryType,
  WalletReferenceType,
  WalletResponse,
  WalletStatus,
  WalletTransactionResponse,
  WalletTxType,
  WebhookSubscription,
  WorkflowDefinition,
  WorkflowInstance,
  WorkflowInstanceStatus,
  WorkflowStepStatus,
  ProductResponse,
  OrderItemResponse,
  OrderResponse,
  OrderStatsResponse,
  PaymentStatus,
  PaymentOrderResponse,
  PaymentChannelResponse,
  PaymentTransactionResponse,
  PaymentRefundResponse,
  AvailableChannelItem,
  AvailableChannelsResponse,
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
export type PaymentOrder = PaymentOrderResponse;
export type PaymentChannel = PaymentChannelResponse;
export type PaymentTransaction = PaymentTransactionResponse;
export type PaymentRefund = PaymentRefundResponse;
export type Order = OrderResponse;
export type OrderItem = OrderItemResponse;
export type OrderStats = OrderStatsResponse;
export type Product = ProductResponse;

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

export interface TrendPoint {
  date: string;
  count: number;
}

export interface RouteInfo {
  method: string;
  path: string;
  source: string;
  source_name: string;
}
