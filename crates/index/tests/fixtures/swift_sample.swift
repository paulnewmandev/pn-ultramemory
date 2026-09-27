// SPDX-License-Identifier: Apache-2.0
import Foundation

/// A shape that has an area.
public protocol Shape {
    func area() -> Double
    var name: String { get }
}

/// A circle.
struct Circle: Shape {
    var radius: Double

    /// Computes the area.
    func area() -> Double {
        return Double.pi * radius * radius
    }

    private func secret() {
        helper()
    }

    static func unit() -> Circle { Circle(radius: 1) }
}

@MainActor
final class Model<T>: ObservableObject {
    private var items: [T] = []

    init(items: [T]) {
        self.items = items
    }

    public func append(_ item: T) {
        items.append(item)
        notify(item)
    }
}

extension Circle {
    func scaled(by factor: Double) -> Circle {
        return Circle(radius: radius * factor)
    }
}

enum Direction {
    case north, south

    func opposite() -> Direction {
        switch self {
        case .north: return .south
        case .south: return .north
        }
    }
}

func topLevel(_ value: Int) -> Int {
    return value + compute(value)
}
