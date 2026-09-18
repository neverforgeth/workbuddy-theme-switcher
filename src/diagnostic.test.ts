import { expect, it } from "vitest";
import { errorText } from "./studio-api";

it("manual error notifications retain a bounded error code", () => {
  expect(errorText({ code: "CODEDROBE_THEME_READ_FAILED", message: "资源无法读取" }))
    .toBe("资源无法读取（CODEDROBE_THEME_READ_FAILED）");
  expect(errorText(new Error("网络暂不可用"))).toBe("网络暂不可用");
  expect(errorText({ code: "PRIVATE\nC:/user", message: "失败" })).toBe("失败");
});
