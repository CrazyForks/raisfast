import { api, type PaginatedData } from "./api";

const PLUGIN_BASE = "/plugins/crm";
const CMS_BASE = "/cms";

export interface Company {
  id: string;
  name: string;
  website?: string;
  industry?: string;
  size?: string;
  phone?: string;
  address?: string;
  city?: string;
  country?: string;
  description?: string;
  owner_id?: string;
  created_at: string;
  updated_at?: string;
}

export interface Contact {
  id: string;
  first_name: string;
  last_name: string;
  email?: string;
  phone?: string;
  mobile?: string;
  job_title?: string;
  company?: string;
  source?: string;
  status?: string;
  lifecycle_stage?: string;
  owner_id?: string;
  notes?: string;
  created_at: string;
  updated_at?: string;
}

export interface Deal {
  id: string;
  title: string;
  amount?: number;
  currency?: string;
  stage: string;
  probability?: number;
  contact?: string;
  company?: string;
  owner_id?: string;
  close_date?: string;
  description?: string;
  loss_reason?: string;
  created_at: string;
  updated_at?: string;
}

export interface Activity {
  id: string;
  type?: string;
  subject?: string;
  content?: string;
  contact?: string;
  company?: string;
  deal?: string;
  owner_id?: string;
  activity_date?: string;
  duration_minutes?: number;
  outcome?: string;
  created_at: string;
  updated_at?: string;
}

export interface Note {
  id: string;
  content: string;
  contact?: string;
  company?: string;
  deal?: string;
  owner_id?: string;
  pinned?: number;
  created_at: string;
  updated_at?: string;
}

export interface PipelineStage {
  stage: string;
  deals: Deal[];
  total_amount: number;
  weighted_amount: number;
  count: number;
}

export interface PipelineData {
  stages: PipelineStage[];
  total_value: number;
  weighted_value: number;
}

export interface DealDetail extends Deal {
  activities: Activity[];
  notes: Note[];
}

export interface TimelineEvent {
  id: string;
  type: string;
  subject?: string;
  content?: string;
  created_at: string;
}

export interface CrmStats {
  total_companies: number;
  total_contacts: number;
  total_deals: number;
  open_deals: number;
  won_deals: number;
  total_pipeline_value: number;
  weighted_pipeline_value: number;
  win_rate: number;
  avg_deal_size: number;
  total_activities: number;
  activities_this_week: number;
}

export interface LeaderboardEntry {
  owner_id: string;
  owner_name?: string;
  deal_count: number;
  total_value: number;
  won_value: number;
}

export interface FunnelReport {
  stages: { stage: string; count: number; value: number; conversion_rate: number }[];
}

export interface ActivityReport {
  total: number;
  by_type: Record<string, number>;
  by_outcome: Record<string, number>;
  by_owner: { owner_id: string; count: number }[];
}

export const crm = {
  listCompanies: (page = 1, pageSize = 50) =>
    api.get<PaginatedData<Company>>(
      `${CMS_BASE}/companies?page=${page}&page_size=${pageSize}`,
    ),

  getCompany: (id: string) =>
    api.get<Company>(`${CMS_BASE}/companies/${id}`),

  createCompany: (data: Partial<Company>) =>
    api.post<Company>(`${CMS_BASE}/companies`, data),

  updateCompany: (id: string, data: Partial<Company>) =>
    api.put<Company>(`${CMS_BASE}/companies/${id}`, data),

  deleteCompany: (id: string) =>
    api.delete(`${CMS_BASE}/companies/${id}`),

  listContacts: (page = 1, pageSize = 50) =>
    api.get<PaginatedData<Contact>>(
      `${CMS_BASE}/contacts?page=${page}&page_size=${pageSize}`,
    ),

  getContact: (id: string) =>
    api.get<Contact>(`${CMS_BASE}/contacts/${id}`),

  createContact: (data: Partial<Contact>) =>
    api.post<Contact>(`${CMS_BASE}/contacts`, data),

  updateContact: (id: string, data: Partial<Contact>) =>
    api.put<Contact>(`${CMS_BASE}/contacts/${id}`, data),

  deleteContact: (id: string) =>
    api.delete(`${CMS_BASE}/contacts/${id}`),

  listDeals: (page = 1, pageSize = 50) =>
    api.get<PaginatedData<Deal>>(
      `${CMS_BASE}/deals?page=${page}&page_size=${pageSize}`,
    ),

  getDeal: (id: string) =>
    api.get<Deal>(`${CMS_BASE}/deals/${id}`),

  createDeal: (data: Partial<Deal>) =>
    api.post<Deal>(`${CMS_BASE}/deals`, data),

  updateDeal: (id: string, data: Partial<Deal>) =>
    api.put<Deal>(`${CMS_BASE}/deals/${id}`, data),

  deleteDeal: (id: string) =>
    api.delete(`${CMS_BASE}/deals/${id}`),

  listActivities: (page = 1, pageSize = 50) =>
    api.get<PaginatedData<Activity>>(
      `${CMS_BASE}/activities?page=${page}&page_size=${pageSize}`,
    ),

  createActivity: (data: Partial<Activity>) =>
    api.post<Activity>(`${CMS_BASE}/activities`, data),

  updateActivity: (id: string, data: Partial<Activity>) =>
    api.put<Activity>(`${CMS_BASE}/activities/${id}`, data),

  deleteActivity: (id: string) =>
    api.delete(`${CMS_BASE}/activities/${id}`),

  listNotes: (page = 1, pageSize = 50) =>
    api.get<PaginatedData<Note>>(
      `${CMS_BASE}/notes?page=${page}&page_size=${pageSize}`,
    ),

  createNote: (data: Partial<Note>) =>
    api.post<Note>(`${CMS_BASE}/notes`, data),

  updateNote: (id: string, data: Partial<Note>) =>
    api.put<Note>(`${CMS_BASE}/notes/${id}`, data),

  deleteNote: (id: string) =>
    api.delete(`${CMS_BASE}/notes/${id}`),

  getPipeline: () =>
    api.get<PipelineData>(`${PLUGIN_BASE}/pipeline`),

  getDealDetail: (dealId: string) =>
    api.get<DealDetail>(`${PLUGIN_BASE}/pipeline/${dealId}`),

  advanceDealStage: (dealId: string, stage: string) =>
    api.post<{ id: string; stage: string }>(`${PLUGIN_BASE}/deals/${dealId}/stage`, { stage }),

  getContactTimeline: (contactId: string) =>
    api.get<TimelineEvent[]>(`${PLUGIN_BASE}/contacts/${contactId}/timeline`),

  getCompanyTimeline: (companyId: string) =>
    api.get<TimelineEvent[]>(`${PLUGIN_BASE}/companies/${companyId}/timeline`),

  getStats: () =>
    api.get<CrmStats>(`${PLUGIN_BASE}/stats`),

  getLeaderboard: () =>
    api.get<LeaderboardEntry[]>(`${PLUGIN_BASE}/leaderboard`),

  convertContactLifecycle: (contactId: string, stage: string) =>
    api.post<{ id: string; lifecycle_stage: string }>(
      `${PLUGIN_BASE}/contacts/${contactId}/convert`,
      { lifecycle_stage: stage },
    ),

  getFunnelReport: () =>
    api.get<FunnelReport>(`${PLUGIN_BASE}/reports/funnel`),

  getActivityReport: () =>
    api.get<ActivityReport>(`${PLUGIN_BASE}/reports/activities`),
};
