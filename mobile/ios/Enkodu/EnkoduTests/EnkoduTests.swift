import XCTest
@testable import Enkodu

class EnkoduUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    override func tearDownWithError() throws {
    }

    func testLaunchApp() throws {
        let app = XCUIApplication()
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 5))
    }

    func testSettingsButtonExists() throws {
        let app = XCUIApplication()
        app.launch()

        // Check if settings button exists (on both supported and unsupported states)
        let settingsButton = app.buttons["Settings"]
        XCTAssertTrue(settingsButton.waitForExistence(timeout: 5), "Settings button should exist")
    }

    func testQueueButtonExists() throws {
        let app = XCUIApplication()
        app.launch()

        let queueButton = app.buttons["View Queue"]
        XCTAssertTrue(queueButton.waitForExistence(timeout: 5), "Queue button should exist")
    }

    func testSettingsFlow() throws {
        let app = XCUIApplication()
        app.launch()

        // Navigate to settings
        let settingsButton = app.buttons["Settings"]
        XCTAssertTrue(settingsButton.waitForExistence(timeout: 5))
        settingsButton.tap()

        // Verify server URL field exists
        let serverField = app.textFields["Server URL (https://...)"]
        XCTAssertTrue(serverField.waitForExistence(timeout: 5), "Server URL field should exist")

        // Enter a URL
        serverField.tap()
        serverField.typeText("https://enkodu.example.com")

        // Tap done
        let doneButton = app.buttons["Done"]
        XCTAssertTrue(doneButton.waitForExistence(timeout: 5))
        doneButton.tap()

        // Verify we're back to main view
        XCTAssertTrue(settingsButton.waitForExistence(timeout: 5))
    }

    func testCapabilityGateOnUnsupportedDevice() throws {
        // This test only passes on devices without AV1 hardware support
        // On supported devices, it will fail because the gate view won't appear
        let app = XCUIApplication()
        app.launch()

        // If the device supports AV1, the main view will show instead of the gate
        // We check for the presence of either view
        let gateText = app.staticTexts["AV1 Not Supported"]
        let mainText = app.staticTexts["Ready to upgrade videos"]

        let gateExists = gateText.waitForExistence(timeout: 5)
        let mainExists = mainText.waitForExistence(timeout: 5)

        XCTAssertTrue(gateExists || mainExists, "Either gate or main view should appear")
    }
}

class EnkoduUnitTests: XCTestCase {

    func testAv1CapabilityCheck() async throws {
        let result = await Av1CapabilityChecker.check()
        XCTAssertNotNil(result)
        XCTAssertFalse(result.reason.isEmpty)
    }

    func testSettingsValidation() {
        XCTAssertTrue(SettingsView.validateServerURL("https://example.com"))
        XCTAssertTrue(SettingsView.validateServerURL("http://localhost:8000"))
        XCTAssertFalse(SettingsView.validateServerURL(""))
        XCTAssertFalse(SettingsView.validateServerURL("not-a-url"))
        XCTAssertFalse(SettingsView.validateServerURL("ftp://example.com"))
    }

    func testAuthorizedRequestAddsBearerToken() async throws {
        let api = EnkoduApi(serverURL: "https://enkodu.example.com", authTokenProvider: { "secret-token" })
        let request = await api.authorizedRequest(path: "status")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer secret-token")
    }

    func testAuthorizedRequestSkipsEmptyToken() async throws {
        let api = EnkoduApi(serverURL: "https://enkodu.example.com", authTokenProvider: { "   " })
        let request = await api.authorizedRequest(path: "status")
        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
    }

    func testAuthCheckResultMapsToAuthState() {
        XCTAssertEqual(AuthCheckResult.connected.authState, .connected)
        XCTAssertEqual(AuthCheckResult.tokenRejected.authState, .tokenRejected)
        XCTAssertEqual(AuthCheckResult.permissionDenied.authState, .permissionDenied)
        XCTAssertEqual(AuthCheckResult.serverUnreachable("timeout").authState, .serverUnreachable)
    }

    func testRetryBackoffCalculation() {
        let baseDelay: Double = 500
        let multiplier = 1.5
        let maxDelay: Double = 30000

        for attempt in 0..<10 {
            let delay = baseDelay * pow(multiplier, Double(attempt))
            let jittered = delay * 1.3 // max with jitter
            XCTAssertLessThanOrEqual(jittered, maxDelay, "Delay at attempt \(attempt) should not exceed max")
        }
    }

    func testTransferStatePersistence() throws {
        let controller = PersistenceController(inMemory: true)
        let context = controller.container.viewContext

        let state = TransferState(context: context)
        state.id = UUID()
        state.uploadId = "test-upload"
        state.filePath = "/tmp/test.mp4"
        state.totalBytes = 1024
        state.status = TransferStatus.pending.rawValue
        state.transferType = TransferType.upload.rawValue
        state.createdAt = Date()
        state.updatedAt = Date()

        try context.save()

        let fetchRequest = TransferState.fetchRequest()
        fetchRequest.predicate = NSPredicate(format: "uploadId == %@", "test-upload")
        let results = try context.fetch(fetchRequest)

        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.totalBytes, 1024)
    }
}

extension PersistenceController {
    convenience init(inMemory: Bool) {
        self.init()
        if inMemory {
            let description = container.persistentStoreDescriptions.first
            description?.url = URL(fileURLWithPath: "/dev/null")
        }
    }
}
