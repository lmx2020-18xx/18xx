/// Variable-length bitfield stored as Vec<u32>.
/// Each u32 holds 32 bits. Bit N is in entry N/32 at position N%32.
#[derive(Debug, Clone, Default)]
pub struct Bitfield(Vec<u32>);

impl Bitfield {
    pub fn new() -> Self {
        Bitfield(vec![0])
    }

    /// Set bit at given position, expanding the vector if needed.
    pub fn set(&mut self, bit: usize) {
        let entry = bit / 32;
        let mask = 1u32 << (bit & 31);
        while self.0.len() <= entry {
            self.0.push(0);
        }
        self.0[entry] |= mask;
    }

    /// Check if any bits overlap between self and other (bitwise AND != 0).
    pub fn conflicts(&self, other: &Bitfield) -> bool {
        let min_len = self.0.len().min(other.0.len());
        for i in 0..min_len {
            if (self.0[i] & other.0[i]) != 0 {
                return true;
            }
        }
        false
    }

    /// Merge two bitfields (bitwise OR), returning a new bitfield.
    pub fn merge(&self, other: &Bitfield) -> Bitfield {
        let max_len = self.0.len().max(other.0.len());
        let mut result = Vec::with_capacity(max_len);
        for i in 0..max_len {
            let a = self.0.get(i).copied().unwrap_or(0);
            let b = other.0.get(i).copied().unwrap_or(0);
            result.push(a | b);
        }
        Bitfield(result)
    }

    /// Check if bitfield is all zeros.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&v| v == 0)
    }

    /// Get the raw data for serialization.
    pub fn as_slice(&self) -> &[u32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_conflicts() {
        let mut a = Bitfield::new();
        let mut b = Bitfield::new();

        a.set(0);
        a.set(5);
        b.set(10);
        b.set(20);
        assert!(!a.conflicts(&b));

        b.set(5);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn test_merge() {
        let mut a = Bitfield::new();
        let mut b = Bitfield::new();
        a.set(0);
        a.set(31);
        b.set(32);
        b.set(63);

        let merged = a.merge(&b);
        assert_eq!(merged.0.len(), 2);
        assert_eq!(merged.0[0], a.0[0]);
        assert_eq!(merged.0[1], b.0[1]);
    }

    #[test]
    fn test_multiword_conflicts() {
        let mut a = Bitfield::new();
        let mut b = Bitfield::new();
        a.set(50);
        b.set(50);
        assert!(a.conflicts(&b));

        let mut c = Bitfield::new();
        c.set(51);
        assert!(!a.conflicts(&c));
    }

    #[test]
    fn test_empty() {
        let a = Bitfield::new();
        assert!(a.is_empty());

        let mut b = Bitfield::new();
        b.set(100);
        assert!(!b.is_empty());
    }
}
