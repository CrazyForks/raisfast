import { HttpClient } from "./client";
import { SDKError } from "./errors";
import type {
  ListOptions,
  MutateOptions,
  PaginatedData,
  RequestOptions,
  Revision,
} from "./types";

export class Collection<T = Record<string, unknown>> {
  private readonly http: HttpClient;
  private readonly name: string;
  private readonly prefix: string;

  constructor(http: HttpClient, name: string, admin = false) {
    this.http = http;
    this.name = name;
    this.prefix = admin ? `/admin/cms/${name}` : `/cms/${name}`;
  }

  async getList(
    page = 1,
    pageSize = 25,
    options?: ListOptions,
  ): Promise<PaginatedData<T>> {
    const query: Record<string, string> = {
      page: String(page),
      page_size: String(pageSize),
    };
    if (options?.sort) query.sort = options.sort;
    if (options?.filter) query.filter = options.filter;
    if (options?.search) query.search = options.search;
    if (options?.fields) query.fields = options.fields;
    if (options?.status) query.status = options.status;
    if (options?.expand) query.expand = options.expand;

    return this.http.get<PaginatedData<T>>(this.prefix, { ...options, query });
  }

  async getFullList(options?: ListOptions): Promise<T[]> {
    const all: T[] = [];
    let page = 1;
    while (true) {
      const result = await this.getList(page, 200, options);
      all.push(...result.items);
      if (all.length >= result.total) break;
      page++;
    }
    return all;
  }

  async getOne(idOrSlug: string, options?: RequestOptions): Promise<T> {
    return this.http.get<T>(`${this.prefix}/${idOrSlug}`, options);
  }

  async getFirstListItem(
    filter: string,
    options?: ListOptions,
  ): Promise<T> {
    const result = await this.getList(1, 1, { ...options, filter });
    if (result.items.length === 0) {
      throw new SDKError(
        404,
        `No item found matching filter: ${filter}`,
        404,
      );
    }
    return result.items[0];
  }

  async create(data: Partial<T>, options?: MutateOptions): Promise<T> {
    return this.http.request<T>(this.http.pathForCreate(this.prefix), {
      ...options,
      method: this.http.methodForCreate(),
      body: data,
    });
  }

  async update(
    id: string,
    data: Partial<T>,
    options?: MutateOptions,
  ): Promise<T> {
    return this.http.request<T>(this.http.pathForUpdate(this.prefix, id), {
      ...options,
      method: this.http.methodForUpdate(),
      body: data,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete(this.prefix, id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async upload(
    id: string,
    file: File,
    fileField = "file",
    options?: MutateOptions,
  ): Promise<T> {
    const formData = new FormData();
    formData.append(fileField, file);
    return this.http.request<T>(`${this.prefix}/${id}`, {
      ...options,
      method: "POST",
      body: formData,
    });
  }

  // ─── Revisions (versionable) ───

  async listRevisions(
    id: string,
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Revision>> {
    return this.http.get<PaginatedData<Revision>>(
      `/admin/cms/${this.name}/${id}/revisions`,
      { ...options, query: { page: String(page), page_size: String(pageSize) } },
    );
  }

  async getRevision(
    id: string,
    revisionId: string,
    options?: RequestOptions,
  ): Promise<Revision> {
    return this.http.get<Revision>(
      `/admin/cms/${this.name}/${id}/revisions/${revisionId}`,
      options,
    );
  }

  async restoreRevision(
    id: string,
    revisionId: string,
    options?: RequestOptions,
  ): Promise<T> {
    return this.http.post<T>(
      `/admin/cms/${this.name}/${id}/revisions/${revisionId}/restore`,
      {},
      options,
    );
  }

  async diffRevisions(
    id: string,
    revA: string,
    revB: string,
    options?: RequestOptions,
  ): Promise<Record<string, unknown>> {
    return this.http.get<Record<string, unknown>>(
      `/admin/cms/${this.name}/${id}/revisions/${revA}/diff/${revB}`,
      options,
    );
  }
}
