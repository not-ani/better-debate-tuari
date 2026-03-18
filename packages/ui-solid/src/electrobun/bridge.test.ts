import { beforeEach, expect, mock, test } from "bun:test";

type RpcRequestMock = {
  invokeCore: (payload: { command: string; args: Record<string, unknown> }) => Promise<unknown>;
  openDialog: (_options: unknown) => Promise<unknown>;
  addRootFromDialog: (_payload: Record<string, never>) => Promise<unknown>;
  openPath: (_payload: { path: string }) => Promise<boolean>;
};

const rpcRequest: RpcRequestMock = {
  invokeCore: async () => null,
  openDialog: async () => null,
  addRootFromDialog: async () => null,
  openPath: async () => true,
};

let capturedRpcConfig: { maxRequestTime?: number } | null = null;

class MockElectroview {
  static defineRPC(config: { maxRequestTime?: number }) {
    capturedRpcConfig = config;
    return { request: rpcRequest };
  }

  constructor(_options: unknown) {}
}

mock.module("electrobun/view", () => ({
  Electroview: MockElectroview,
}));

const bridge = await import("./bridge");

beforeEach(() => {
  rpcRequest.invokeCore = async () => null;
  rpcRequest.openDialog = async () => null;
  rpcRequest.addRootFromDialog = async () => null;
  rpcRequest.openPath = async () => true;
});

test("uses extended RPC timeout for long indexing requests", () => {
  expect(capturedRpcConfig?.maxRequestTime).toBe(6 * 60 * 60 * 1000);
});

test("addRootFromDialog normalizes structured responses", async () => {
  rpcRequest.addRootFromDialog = async () => ({
    canonicalPath: "/tmp/demo",
    rootsAfter: [
      {
        path: "/tmp/demo",
        fileCount: 3,
        headingCount: 9,
        addedAtMs: 1,
        lastIndexedMs: 2,
      },
      {
        fileCount: 0,
      },
    ],
  });

  const result = await bridge.addRootFromDialog();
  expect(result).toEqual({
    canonicalPath: "/tmp/demo",
    rootsAfter: [
      {
        path: "/tmp/demo",
        fileCount: 3,
        headingCount: 9,
        addedAtMs: 1,
        lastIndexedMs: 2,
      },
    ],
  });
});

test("addRootFromDialog supports legacy string responses", async () => {
  rpcRequest.addRootFromDialog = async () => "/tmp/legacy";

  const result = await bridge.addRootFromDialog();
  expect(result).toEqual({
    canonicalPath: "/tmp/legacy",
    rootsAfter: [],
  });
});

test("openDialog normalizes object payloads", async () => {
  rpcRequest.openDialog = async () => ({
    first: "/tmp/a",
    second: "/tmp/b",
    ignored: 42,
  });

  const result = await bridge.openDialog({ directory: true });
  expect(result).toEqual(["/tmp/a", "/tmp/b"]);
});
