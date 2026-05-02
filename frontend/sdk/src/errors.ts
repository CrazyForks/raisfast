export class SDKError extends Error {
  readonly code: number;
  readonly status: number;
  readonly url: string;
  readonly response: Record<string, unknown>;
  readonly isAbort: boolean;
  readonly originalError: Error | null;

  constructor(
    code: number,
    message: string,
    status = 400,
    url = "",
    response: Record<string, unknown> = {},
    isAbort = false,
    originalError: Error | null = null,
  ) {
    super(message);
    this.name = "SDKError";
    this.code = code;
    this.status = status;
    this.url = url;
    this.response = response;
    this.isAbort = isAbort;
    this.originalError = originalError;
  }
}
