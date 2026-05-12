import type { PostResponse, CommentResponse } from "@raisfast/sdk";

export type Post = Omit<PostResponse, "id"> & { id: string };
export type Comment = Omit<CommentResponse, "id"> & { id: string };
