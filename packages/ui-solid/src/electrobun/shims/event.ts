import { listenEvent } from "../bridge";

export type UnlistenFn = () => void;

export const listen = listenEvent;
