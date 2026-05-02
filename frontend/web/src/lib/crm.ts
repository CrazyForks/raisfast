import { client } from "./raisfast";
import type { PaginatedData } from "@raisfast/sdk";

const PLUGIN_BASE = "/plugins/crm";

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

const companies = client.collection<Company>("companies");
const contacts = client.collection<Contact>("contacts");
const deals = client.collection<Deal>("deals");
const activities = client.collection<Activity>("activities");
const notes = client.collection<Note>("notes");

export const crm = {
  listCompanies: (page = 1, pageSize = 50) =>
    companies.getList(page, pageSize),

  getCompany: (id: string) =>
    companies.getOne(id),

  createCompany: (data: Partial<Company>) =>
    companies.create(data),

  updateCompany: (id: string, data: Partial<Company>) =>
    companies.update(id, data),

  deleteCompany: (id: string) =>
    companies.delete(id),

  listContacts: (page = 1, pageSize = 50) =>
    contacts.getList(page, pageSize),

  getContact: (id: string) =>
    contacts.getOne(id),

  createContact: (data: Partial<Contact>) =>
    contacts.create(data),

  updateContact: (id: string, data: Partial<Contact>) =>
    contacts.update(id, data),

  deleteContact: (id: string) =>
    contacts.delete(id),

  listDeals: (page = 1, pageSize = 50) =>
    deals.getList(page, pageSize),

  getDeal: (id: string) =>
    deals.getOne(id),

  createDeal: (data: Partial<Deal>) =>
    deals.create(data),

  updateDeal: (id: string, data: Partial<Deal>) =>
    deals.update(id, data),

  deleteDeal: (id: string) =>
    deals.delete(id),

  listActivities: (page = 1, pageSize = 50) =>
    activities.getList(page, pageSize),

  createActivity: (data: Partial<Activity>) =>
    activities.create(data),

  updateActivity: (id: string, data: Partial<Activity>) =>
    activities.update(id, data),

  deleteActivity: (id: string) =>
    activities.delete(id),

  listNotes: (page = 1, pageSize = 50) =>
    notes.getList(page, pageSize),

  createNote: (data: Partial<Note>) =>
    notes.create(data),

  updateNote: (id: string, data: Partial<Note>) =>
    notes.update(id, data),

  deleteNote: (id: string) =>
    notes.delete(id),

  getPipeline: () =>
    client.send<PipelineData>(`${PLUGIN_BASE}/pipeline`),

  getDealDetail: (dealId: string) =>
    client.send<DealDetail>(`${PLUGIN_BASE}/pipeline/${dealId}`),

  advanceDealStage: (dealId: string, stage: string) =>
    client.send<{ id: string; stage: string }>(`${PLUGIN_BASE}/deals/${dealId}/stage`, {
      method: "POST",
      body: { stage },
    }),

  getContactTimeline: (contactId: string) =>
    client.send<TimelineEvent[]>(`${PLUGIN_BASE}/contacts/${contactId}/timeline`),

  getCompanyTimeline: (companyId: string) =>
    client.send<TimelineEvent[]>(`${PLUGIN_BASE}/companies/${companyId}/timeline`),

  getStats: () =>
    client.send<CrmStats>(`${PLUGIN_BASE}/stats`),

  getLeaderboard: () =>
    client.send<LeaderboardEntry[]>(`${PLUGIN_BASE}/leaderboard`),

  convertContactLifecycle: (contactId: string, stage: string) =>
    client.send<{ id: string; lifecycle_stage: string }>(
      `${PLUGIN_BASE}/contacts/${contactId}/convert`,
      { method: "POST", body: { lifecycle_stage: stage } },
    ),

  getFunnelReport: () =>
    client.send<FunnelReport>(`${PLUGIN_BASE}/reports/funnel`),

  getActivityReport: () =>
    client.send<ActivityReport>(`${PLUGIN_BASE}/reports/activities`),
};
