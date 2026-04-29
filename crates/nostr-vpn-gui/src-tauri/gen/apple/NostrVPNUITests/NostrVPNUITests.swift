import XCTest

final class NostrVPNUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testLaunchRendersMainInterface() throws {
        let app = XCUIApplication()
        app.launch()

        let rendered = app.staticTexts["Nostr VPN"].waitForExistence(timeout: 10)
            || app.staticTexts["Status"].exists
            || app.staticTexts["Network 1"].exists

        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = "nostr-vpn-launch"
        attachment.lifetime = .keepAlways
        add(attachment)

        let labels = app.staticTexts.allElementsBoundByIndex.map { $0.label }.joined(separator: "\n")
        let labelsAttachment = XCTAttachment(string: labels)
        labelsAttachment.name = "nostr-vpn-static-text-labels"
        labelsAttachment.lifetime = .keepAlways
        add(labelsAttachment)

        let localhostFailure = app.staticTexts.containing(
            NSPredicate(format: "label CONTAINS[c] %@", "localhost:1420")
        ).firstMatch
        XCTAssertFalse(localhostFailure.exists)
        XCTAssertTrue(rendered)
    }
}
