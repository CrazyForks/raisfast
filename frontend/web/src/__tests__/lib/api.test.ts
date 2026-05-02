import { describe, it, expect } from "vitest";
import { SDKError } from "@raisfast/sdk";

describe("SDKError", () => {
  it("stores code, message, and status", () => {
    const err = new SDKError(40400, "not found", 404);
    expect(err.code).toBe(40400);
    expect(err.message).toBe("not found");
    expect(err.status).toBe(404);
    expect(err).toBeInstanceOf(Error);
    expect(err).toBeInstanceOf(SDKError);
  });
});
