// Canonical Stage 1 example (language-spec.md §23): a PascalCase
// heap/ARC struct, a lowercase inline struct, all four `let`-binding
// value forms, an ARC-domain method call, an owned struct literal
// consumed by an owned parameter, and explicit `return` (no implicit
// tail-expression return from a fn/method body).

struct Dog {
    name String
}

struct point {
    x f32,
    y f32
}

impl Dog {
    new(name String) Dog {
        return Dog { name = name };
    }

    greeting(self) String {
        return `Woof, I am ${self.name}!`;
    }
}

fn describe_point(p point) f32 {
    return p.x + p.y;
}

// An owned (moved), consuming parameter — `d: Dog` — can only be called
// with a value already in the unique-ownership domain (here, an owned
// struct literal at the call site); an ARC-domain `Dog` cannot be passed
// here without an explicit domain conversion (language-spec §9, not yet
// implemented in Stage 1).
fn consume_dog(d: Dog) String {
    return d.name;
}

fn main() {
    let dog Dog = Dog.new("Rex");
    println(dog.greeting());

    let count i32 = 3;
    // Owned inline value — same machine representation as `i32`, but
    // move-only semantically (language-spec §3, §4.4). Use-after-move
    // enforcement is Stage 2; Stage 1 only parses and type-checks this.
    let owned_count: i32 = 3;

    let origin = point { x = 0.0, y = 0.0 };
    println(`sum of coordinates: ${describe_point(origin)}`);

    let consumed = consume_dog(:Dog { name = "Buddy" });
    println(consumed);

    println(`count = ${count}`);
    if owned_count == 3 {
        println("owned_count checks out");
    }
}
