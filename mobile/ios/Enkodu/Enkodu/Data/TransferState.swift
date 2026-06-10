import Foundation
import CoreData

@objc(TransferState)
public class TransferState: NSManagedObject {
    @NSManaged public var id: UUID
    @NSManaged public var uploadId: String?
    @NSManaged public var jobId: String?
    @NSManaged public var filePath: String
    @NSManaged public var localTempPath: String?
    @NSManaged public var totalBytes: Int64
    @NSManaged public var bytesTransferred: Int64
    @NSManaged public var status: String
    @NSManaged public var transferType: String
    @NSManaged public var lastError: String?
    @NSManaged public var retryCount: Int16
    @NSManaged public var createdAt: Date
    @NSManaged public var updatedAt: Date
}

extension TransferState {
    @nonobjc public class func fetchRequest() -> NSFetchRequest<TransferState> {
        return NSFetchRequest<TransferState>(entityName: "TransferState")
    }
}

enum TransferStatus: String {
    case pending = "PENDING"
    case active = "ACTIVE"
    case paused = "PAUSED"
    case failed = "FAILED"
    case done = "DONE"
    case cancelled = "CANCELLED"
}

enum TransferType: String {
    case upload = "UPLOAD"
    case download = "DOWNLOAD"
}
