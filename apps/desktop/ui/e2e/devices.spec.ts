import { expect, test } from "@playwright/test";

/** Device setup against the mock backend: the wizard's tabs and the LAN allow prompt. */

test("the listening status opens device setup with its four tabs", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("listening-status").click();
  const sheet = page.getByTestId("device-setup");
  await expect(sheet).toBeVisible();
  await expect(sheet.getByTestId("device-tab-simulator")).toContainText("iPhone 17 Pro");

  await sheet.getByTestId("device-tab-iphone-trigger").click();
  await expect(sheet.getByTestId("device-tab-iphone")).toContainText("Turn on Allow devices on your network");
  await sheet.getByTestId("lan-access").click();
  await expect(sheet.getByTestId("device-qr")).toContainText("http://192.168.1.20:9090/");

  await sheet.getByTestId("device-tab-android-trigger").click();
  await expect(sheet.getByTestId("device-tab-android")).toContainText("network_security_config.xml");
  await sheet.getByTestId("device-tab-emulator-trigger").click();
  await expect(sheet.getByTestId("emulator-row")).toContainText("emulator-5554");
});

test("a new LAN client asks to be allowed, with a nickname", async ({ page }) => {
  await page.goto("/?devicePrompt=192.168.1.42");
  const prompt = page.getByTestId("device-allow-prompt");
  await expect(prompt).toContainText("Allow 192.168.1.42?");
  await expect(prompt).toContainText("2 connections waiting");
  await page.getByTestId("device-nickname").fill("Pixel 8");
  await page.getByTestId("device-allow").click();
  await expect(prompt).toBeHidden();

  await page.getByTestId("listening-status").click();
  await expect(page.getByTestId("known-devices")).toContainText("Pixel 8 (192.168.1.42)");
});
