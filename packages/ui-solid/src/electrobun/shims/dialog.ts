import { addRootFromDialog, openDialog, type OpenDialogOptions } from "../bridge";

export const open = (options: OpenDialogOptions = {}) =>
  openDialog(options);

export { addRootFromDialog };
