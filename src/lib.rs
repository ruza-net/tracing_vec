mod index;
mod tracing_vec;

pub use tracing_vec::TracingVec;
pub use index::{ TimedIndex, TimelessIndex, IndexError };



#[cfg(test)]
mod tests {
    #[test]
    fn todo() {
        assert_eq!(0, 1, "Add proper tests!");
    }
}
