export interface NavigatorPlatformInfo {
  userAgent: string;
  maxTouchPoints?: number;
}

export interface PlatformInfo {
  isIOS: boolean;
  isMobile: boolean;
}

/** Detect only the two mobile OS families this Tauri app ships. */
export function detectPlatform(navigatorInfo?: NavigatorPlatformInfo): PlatformInfo {
  if (!navigatorInfo) return { isIOS: false, isMobile: false };

  const { userAgent } = navigatorInfo;
  const isIPadDesktopUA = /Macintosh/i.test(userAgent)
    && (navigatorInfo.maxTouchPoints ?? 0) > 1;
  const isIOS = /iPhone|iPad|iPod/i.test(userAgent) || isIPadDesktopUA;
  return {
    isIOS,
    isMobile: isIOS || /Android/i.test(userAgent),
  };
}

const current = detectPlatform(
  typeof navigator === "undefined"
    ? undefined
    : { userAgent: navigator.userAgent, maxTouchPoints: navigator.maxTouchPoints },
);

export const isIOS = current.isIOS;
export const isMobile = current.isMobile;
