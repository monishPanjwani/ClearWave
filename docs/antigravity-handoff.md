# Antigravity implementation handoff

Open workspace `/home/monish/data/coding/project/ClearWave`.

Implement only the **Next phase: Omarchy themes and portable theme menu** section in `docs/linux-redesign-plan.md`. Read that entire section and repository instructions first. The existing Linux audio redesign is already implemented and tested; preserve it and all current uncommitted changes. Do not reset the repository or overwrite unrelated work.

The required result is live following of Omarchy's active color palette, plus a compact in-app theme menu with bundled Omarchy palettes that work offline on other Linux distributions. Follow the plan's source discovery, palette provenance/licensing, light/dark contrast, saved preference, fallback, directory replacement handling, smooth switching, and validation requirements. Keep the interface minimal. Appearance changes must not restart audio, change routing, or activate microphone capture.

Implement the code rather than only producing a proposal. Work only in this repository: do not modify the user's Omarchy configuration or active theme, install services, record a real microphone, or change real audio defaults. Use temporary fixture palettes and generated devices for tests. Do not commit, push, or publish a release.

Run formatting, strict Clippy, both test configurations and the release build as specified in the plan. Inspect actual UI screenshots for light/dark palettes and minimum window size. Update README, provenance/license notices, screenshots and `docs/validation.md`, distinguishing verified results from untested behavior. Mark the theme phase complete in the plan only if it passes its acceptance criteria.

At completion write `docs/antigravity-results.md` listing changed files, implementation decisions, shipped palette names and sources/licenses, exact checks/results, screenshot paths, and remaining defects or unverified checks. Another agent will review the result. Report failures honestly; do not claim tests ran if they did not.
