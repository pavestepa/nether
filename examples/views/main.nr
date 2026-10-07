fn main(): i32 {
    var numbers = {1, 2}
    let {ref var first, ref second} = numbers
    *first += second

    // Once the views are no longer used, the owner can be changed directly.
    numbers[1] = 5
    return numbers[0] + numbers[1]
}
