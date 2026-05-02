import { client } from "@/lib/raisfast";

const BASE = "/plugins/forum";

async function pluginGet<T>(path: string): Promise<T | null> {
  return client.send<T | null>(BASE + path);
}

async function pluginPost<T>(path: string, body?: object): Promise<T> {
  return client.send<T>(BASE + path, { method: "POST", body });
}

async function pluginPut<T>(path: string, body?: object): Promise<T> {
  return client.send<T>(BASE + path, { method: "PUT", body });
}

async function pluginDelete<T>(path: string, body?: object): Promise<T> {
  return client.send<T>(BASE + path, {
    method: "DELETE",
    body,
  });
}

export interface ForumBoard {
  id: string;
  name: string;
  slug: string;
  description?: string;
  icon?: string;
  topic_count: number;
  post_count: number;
  last_activity_at?: string;
  parent_id?: string;
  parent_name?: string;
  sort_order: number;
  children?: ForumBoard[];
}

export interface ForumTopic {
  id: string;
  title: string;
  slug?: string;
  content?: string;
  board_id?: string;
  author_id: string;
  author_name?: string;
  author_avatar?: string;
  reply_count: number;
  view_count: number;
  is_pinned: boolean;
  is_locked: boolean;
  is_solved: boolean;
  tags?: string;
  last_reply_at?: string;
  last_reply_user_id?: string;
  board_name?: string;
  board_slug?: string;
  created_at: string;
  updated_at?: string;
  is_owner?: boolean;
}

export interface ForumReply {
  id: string;
  content: string;
  topic_id: string;
  author_id: string;
  author_name?: string;
  author_avatar?: string;
  parent_reply_id?: string;
  vote_count: number;
  is_answer: boolean;
  created_at: string;
  updated_at?: string;
}

export interface PaginatedResult<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export interface BoardTopicsResult extends PaginatedResult<ForumTopic> {
  board_id: string;
}

export interface PollOption {
  id: string;
  text: string;
  vote_count: number;
  sort_order: number;
}

export interface Poll {
  id: string;
  topic_id: string;
  question: string;
  max_choices: number;
  is_closed: boolean;
  options: PollOption[];
  total_votes: number;
  user_votes: string[];
  created_at: string;
}

export interface PollCreateParams {
  topic_id: string;
  question: string;
  options: string[];
  max_choices?: number;
}

export const forum = {
  listBoards: () =>
    client.send<PaginatedResult<ForumBoard>>("/cms/forum_boards").then((d) => d.items),

  listBoardTopics: async (slug: string, page = 1, pageSize = 20) => {
    const boards = await client.send<PaginatedResult<ForumBoard>>("/cms/forum_boards?page_size=100").then((d) => d.items);
    const board = boards.find((b: ForumBoard) => b.slug === slug);
    if (!board) throw new Error("Board not found");
    return client.send<BoardTopicsResult>(
      `/cms/forum_topics?page=${page}&page_size=${pageSize}&board=${board.id}`,
    ).then((result) => ({ ...result, board_id: board.id }));
  },

  getTopic: (id: string) =>
    client.send<ForumTopic>(`/cms/forum_topics/${id}`),

  createTopic: (data: { title: string; content: string; board: string; author_id: string; tags?: string }) =>
    client.send<ForumTopic>("/cms/forum_topics", { method: "POST", body: data }),

  createReply: (data: { content: string; topic: string; author_id: string; parent_reply?: string }) =>
    client.send<ForumReply>("/cms/forum_replies", { method: "POST", body: data }),

  acceptAnswer: (userId: string, replyId: string) =>
    pluginPut<{ id: string; is_answer: boolean }>(`/replies/${replyId}/accept`, { user_id: userId }),

  vote: (userId: string, targetType: string, targetId: string, value: number) =>
    pluginPost<{ target_type: string; target_id: string; value: number }>("/vote", {
      user_id: userId,
      target_type: targetType,
      target_id: targetId,
      value,
    }),

  unvote: (userId: string, targetType: string, targetId: string) =>
    pluginDelete<{ removed: true }>("/vote", {
      user_id: userId,
      target_type: targetType,
      target_id: targetId,
    }),

  createPoll: (userId: string, data: Omit<PollCreateParams, "user_id">) =>
    pluginPost<Poll>("/polls", { user_id: userId, ...data }),

  getPoll: (topicId: string, userId?: string) => {
    const query = userId ? `?user_id=${encodeURIComponent(userId)}` : "";
    return pluginGet<Poll | null>(`/polls/${topicId}${query}`);
  },

  castVote: (userId: string, pollId: string, optionIds: string[]) =>
    pluginPost<{ poll_id: string; voted_options: string }>(`/polls/${pollId}/vote`, {
      user_id: userId,
      option_ids: optionIds,
    }),

  deletePoll: (userId: string, pollId: string) =>
    pluginDelete<{ deleted: true }>(`/polls/${pollId}`, { user_id: userId }),

  updateTopic: (id: string, data: Record<string, unknown>) =>
    client.send<ForumTopic>(`/cms/forum_topics/${id}`, { method: "PUT", body: data }),

  deleteTopic: (id: string) =>
    client.send(`/cms/forum_topics/${id}`, { method: "DELETE" }),

  deleteReply: (id: string) =>
    client.send(`/cms/forum_replies/${id}`, { method: "DELETE" }),
};
