import { describe, it, expect } from "vitest";
import { ApiError } from "@/lib/api";

describe("ApiError", () => {
  it("stores code and message", () => {
    const err = new ApiError(40400, "not found");
    expect(err.code).toBe(40400);
    expect(err.message).toBe("not found");
    expect(err).toBeInstanceOf(Error);
    expect(err).toBeInstanceOf(ApiError);
  });
});
