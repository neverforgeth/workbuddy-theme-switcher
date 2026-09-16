import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { SettingsPanel } from "./SettingsPanel";
import { studioApi } from "./studio-api";

afterEach(cleanup);
it("only clears the application's historical credential after explicit confirmation", async () => {
  const api = {
    ...studioApi,
    clearLegacyAiCredential: vi.fn(async () => undefined),
  };
  render(
    <SettingsPanel api={api} snapshot={null} hidden={false} notify={vi.fn()} />,
  );
  expect(api.clearLegacyAiCredential).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "清除旧 AI 凭据" }));
  expect(api.clearLegacyAiCredential).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "确认清除" }));
  await waitFor(() =>
    expect(api.clearLegacyAiCredential).toHaveBeenCalledWith(true),
  );
});
it("offers local diagnostics without an AI endpoint, model or key form", () => {
  render(
    <SettingsPanel
      api={studioApi}
      snapshot={null}
      hidden={false}
      notify={vi.fn()}
    />,
  );
  expect(screen.queryByRole("textbox", { name: "接口地址" })).toBeNull();
  expect(screen.queryByRole("textbox", { name: "视觉模型" })).toBeNull();
  expect(screen.queryByLabelText("API Key")).toBeNull();
  expect(screen.getByRole("heading", { name: "WorkBuddy 连接" })).toBeTruthy();
});
