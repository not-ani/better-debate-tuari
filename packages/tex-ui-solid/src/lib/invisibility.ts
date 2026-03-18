export type InvisibilityMode = boolean;

const INVISIBILITY_STORAGE_KEY = "tex-invisibility-mode";

export const getInitialInvisibilityMode = (): InvisibilityMode => {
  if (typeof window === "undefined") {
    return false;
  }

  return window.localStorage.getItem(INVISIBILITY_STORAGE_KEY) === "1";
};

export const persistInvisibilityMode = (enabled: InvisibilityMode) => {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(INVISIBILITY_STORAGE_KEY, enabled ? "1" : "0");
};
