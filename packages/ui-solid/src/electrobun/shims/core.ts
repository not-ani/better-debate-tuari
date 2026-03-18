import { invokeCore } from "../bridge";

export const invoke = <T>(command: string, args?: Record<string, unknown>) =>
  invokeCore<T>(command, args);
