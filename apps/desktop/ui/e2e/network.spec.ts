import { expect, test } from "@playwright/test";

/**
 * Settings › Network (M18) against the mock backend: a draft saved as a
 * whole, the engine's refusals shown in the Save bar, and passwords that
 * never come back. The dialling itself is tested in Rust
 * (`crates/pm-core/tests/network.rs`).
 */
test("network settings save as a whole and explain what they refuse", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("settings-button").click();
  await page.getByTestId("settings-page-network").click();
  const network = page.getByTestId("settings-network");

  // The saved password is shown as saved, never as text.
  const rule = network.getByTestId("network-upstream-0");
  await expect(rule.getByPlaceholder("Saved in the keychain")).toHaveValue("");
  await expect(network.getByTestId("network-socks")).toContainText("socks5h://127.0.0.1:9090");
  await expect(network.getByTestId("network-save")).toBeDisabled();

  // A reverse proxy on the proxy's own port is refused, and says why.
  await network.getByTestId("network-add-reverse").click();
  const reverse = network.getByTestId("network-reverse").getByTestId("network-row").nth(1);
  await reverse.locator("input[type=number]").fill("9090");
  await network.getByTestId("network-save").click();
  await expect(network.getByTestId("settings-error")).toContainText("the proxy's own port");

  // Fixed, it saves and starts listening.
  await reverse.locator("input[type=number]").fill("8081");
  await network.getByTestId("network-add-dns").click();
  await network.getByTestId("network-dns").getByPlaceholder("api.example.com or *.staging.test").last().fill("*.local.test");
  await network.getByTestId("network-save").click();
  await expect(network.getByTestId("settings-error")).toHaveCount(0);
  await expect(network.getByTestId("network-save")).toBeDisabled();
  await expect(network.getByTestId("network-reverse")).toContainText("http://127.0.0.1:8081 → http://localhost:3000");

  // Leaving and coming back reads what was saved.
  await page.getByTestId("settings-page-proxy").click();
  await page.getByTestId("settings-page-network").click();
  await expect(network.getByTestId("network-dns").getByTestId("network-row")).toHaveCount(2);
});
