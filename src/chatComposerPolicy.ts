export const CHAT_COMPOSER_MIN_ROWS = 1;
export const CHAT_COMPOSER_MAX_ROWS = 6;
export const CHAT_COMPOSER_LINE_HEIGHT_PX = 21;
export const CHAT_COMPOSER_VERTICAL_PADDING_PX = 18;
export const CHAT_COMPOSER_MIN_HEIGHT_PX = CHAT_COMPOSER_LINE_HEIGHT_PX + CHAT_COMPOSER_VERTICAL_PADDING_PX;
export const CHAT_COMPOSER_MAX_HEIGHT_PX = CHAT_COMPOSER_LINE_HEIGHT_PX * CHAT_COMPOSER_MAX_ROWS
  + CHAT_COMPOSER_VERTICAL_PADDING_PX;
export const CHAT_MESSAGE_MAX_CHARS = 20_000;

export interface ComposerKeyInput {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
}

export function shouldSendFromComposer(input: ComposerKeyInput): boolean {
  return input.key === "Enter" && !input.shiftKey && !input.isComposing;
}

export function clampComposerHeight(scrollHeight: number): number {
  return Math.min(
    CHAT_COMPOSER_MAX_HEIGHT_PX,
    Math.max(CHAT_COMPOSER_MIN_HEIGHT_PX, scrollHeight),
  );
}

export function canSubmitChatMessage(value: string, active: boolean): boolean {
  return !active && value.trim().length > 0;
}
