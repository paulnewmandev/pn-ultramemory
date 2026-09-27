// SPDX-License-Identifier: Apache-2.0
// Fixture for the extraction tests: a small geometry library in C++.
#include <vector>
#include <memory>
#include "shape.hpp"

using namespace std;
using Points = std::vector<int>;

namespace geo {
namespace detail {

/// Clamps a value.
template <typename T>
T clamp(T value, T low, T high) {
    return value < low ? low : (value > high ? high : value);
}

}  // namespace detail

/// Base class of all shapes.
template <typename T>
class Shape : public Base<T>, private Named {
public:
    Shape() = default;
    virtual ~Shape();

    /// Area of the shape.
    virtual double area() const = 0;

    static int count();

    int size() const { return helper(cached_) + this->extra(); }

    Shape &operator=(const Shape &other);

protected:
    void update();

private:
    int cached_;
    void hidden();
};

struct Point {
    int x;
    int y;
    double norm() const { return std::sqrt(x * x + y * y); }
};

enum class Mode { Fast, Precise };

/// A circle.
class Circle : public Shape<double> {
public:
    explicit Circle(double r) : radius_(r) {}
    double area() const override;

private:
    double radius_;
};

double Circle::area() const {
    return 3.14159 * radius_ * radius_;
}

void Shape::hidden() {
    detail::clamp(1, 0, 2);
    auto p = new Point();
    logger.info("hidden");
}

static int internal_only(int v) { return v + 1; }

int total(const Points &points) {
    int sum = 0;
    for (int p : points) sum += p;
    return sum;
}

}  // namespace geo

int main(int argc, char **argv) {
    geo::Circle c(2.0);
    return static_cast<int>(c.area());
}
