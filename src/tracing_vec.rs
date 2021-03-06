use crate::index::*;
use serde::{ Serialize, Deserialize };



/// Tracing vector is a vector that manages its history by keeping track of its elements.
///
/// # Storage
/// The elements are held in a common unstructured storage, together with information about their
/// lifetimes.
///
/// # Versioning
/// A tracing vector contains a list of versions, each containing a vector of identifiers which
/// point to the data in storage. When the tracing vector needs to move the elements, it creates a
/// new version encoding the new ordering.
///
/// # Comparison with VersionedVec
/// A tracing vector has two major benefits: it doesn't need to `clone` its contents when it moves
/// elements, and it can convert a pseudotime reference to an absolute index, even when the
/// pseudotime is long obsolete.
///
/// While a versioned vector cannot do that (since it cannot keep track of the movement of then
/// elements), it has faster lookup: first lookup the version, then the data. A tracing vector has
/// to lookup the version, the ID and then the data. However, if one doesn't need to support
/// aliasing, one can take a reference to the data itself. This index then performs just one
/// lookup.
///
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TracingVec<X> {
    mem: Vec<Trace<X>>,
    snapshots: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Trace<X> {
    pub val: X,
    pub birth: usize,
}


impl<X> Default for TracingVec<X> {
    fn default() -> Self {
        Self::new()
    }
}

impl<X> From<Vec<X>> for TracingVec<X> {
    fn from(mem: Vec<X>) -> Self {
        let (v, mem) = mem
            .into_iter()
            .map(|val| Trace { val, birth: 0 })
            .enumerate()
            .unzip();

        Self {
            mem,

            snapshots: vec![v],
        }
    }
}



// IMPL: Initialization
//
impl<X> TracingVec<X> {
    pub fn new() -> Self {
        Self {
            mem: vec![],
            snapshots: vec![vec![]],
        }
    }

    pub unsafe fn from_raw_parts(mem: Vec<X>, snapshots: Vec<Vec<usize>>) -> Self {
        Self {
            snapshots,
            mem: mem
                .into_iter()
                .map(|val| Trace { val, birth: 0 })
                .collect(),
        }
    }
}

// IMPL: Additive Mutations
//
impl<X> TracingVec<X> {
    pub fn push(&mut self, val: X) {
        let birth = self.pseudotime();

        self.mem.push(Trace { val, birth });

        let tracing_index = self.last_obj_index();

        self.latest_snapshot_mut().push(tracing_index);
    }
}

// IMPL: Subtractive Mutations
//
impl<X> TracingVec<X> {
    pub fn pop(&mut self) -> Option<TimelessIndex> {
        self.new_snapshot()
            .pop()
            .map(|pos| TimelessIndex { pos })
    }

    #[track_caller]
    pub fn insert_before(&mut self, index: impl Into<TracingIndex>, val: X) -> TimedIndex {
        self.try_insert_before(index, val)
            .unwrap()
    }

    pub fn try_insert_before(&mut self, index: impl Into<TracingIndex>, val: X) -> Result<TimedIndex, IndexError> {
        let absolute_index = self.latest_index(index)?;

        self.try_insert(absolute_index, val)
    }

    #[track_caller]
    pub fn insert_after(&mut self, index: impl Into<TracingIndex>, val: X) -> TimedIndex {
        self.try_insert_after(index, val)
            .unwrap()
    }

    pub fn try_insert_after(&mut self, index: impl Into<TracingIndex>, val: X) -> Result<TimedIndex, IndexError> {
        let absolute_index = self.latest_index(index)? + 1;

        self.try_insert(absolute_index, val)
    }

    #[track_caller]
    pub fn remove(&mut self, index: impl Into<TracingIndex>) -> TimelessIndex {
        self.try_remove(index)
            .unwrap()
    }

    pub fn try_remove(&mut self, index: impl Into<TracingIndex>) -> Result<TimelessIndex, IndexError> {
        let absolute_index = self.latest_index(index)?;

        let pos =
        self.new_snapshot()
            .remove(absolute_index);

        Ok(TimelessIndex { pos })
    }

    #[track_caller]
    pub fn replace(&mut self, indices: Vec<impl Into<TracingIndex>>, val: X) -> Vec<TimelessIndex> {
        self.try_replace(indices, val).unwrap()
    }

    pub fn try_replace(&mut self, indices: Vec<impl Into<TracingIndex>>, val: X) -> Result<Vec<TimelessIndex>, IndexError> {
        let (indices, removed) = self.try_remove_all(indices)?;

        let birth = self.pseudotime();

        self.mem.push(Trace { val, birth });
        let tracing_index = self.last_obj_index();

        self.snapshots
            .last_mut()
            .unwrap()
            .insert(*indices.first().unwrap(), tracing_index);

        Ok(removed)
    }

    fn try_remove_all(&mut self, indices: Vec<impl Into<TracingIndex>>) -> Result<(Vec<usize>, Vec<TimelessIndex>), IndexError> {
        if indices.is_empty() {
            return Err(IndexError::NoIndicesProvided);
        }

        let mut absolute_indices = vec![];

        // NOTE: Enumerated because we have to update the separate positions ...
        //
        for (position, index) in indices.into_iter().enumerate() {
            let abs = self.latest_index(index)?;

            absolute_indices.push((position, abs));
        }


        let mut last_pos = absolute_indices.first().unwrap().1;
        let mut interleaved_positions = vec![last_pos];

        for (_, abs) in absolute_indices.iter().copied() {
            for pos in last_pos .. abs {
                interleaved_positions.push(pos);
            }

            last_pos = abs + 1;
        }

        absolute_indices.sort_by(|a, b| a.1.cmp(&b.1));

        let mut removed = vec![None; absolute_indices.len()];
        let snapshot = self.new_snapshot();

        for (position, index) in absolute_indices.into_iter().rev() {
            let pos = snapshot.remove(index);

            // ... here.
            removed[position] = Some(TimelessIndex { pos });
        }

        Ok((interleaved_positions, removed.into_iter().map(Option::unwrap).collect()))
    }

    // NOTE: Returns the newly added index.
    //
    fn try_insert(&mut self, absolute_index: usize, val: X) -> Result<TimedIndex, IndexError> {
        let birth = self.pseudotime() + 1;

        self.mem.push(Trace { val, birth });

        let tracing_index = self.last_obj_index();

        self.new_snapshot()
            .insert(absolute_index, tracing_index);

        let pos = absolute_index;
        let pseudotime = birth;

        Ok(TimedIndex { pos, pseudotime })
    }
}

impl<X: Clone> TracingVec<X> {
    pub fn replace_with(&mut self, indices: Vec<impl Into<TracingIndex>>, f: impl FnOnce(Vec<X>) -> X) -> TimedIndex {
        self.try_replace_with(indices, f).unwrap()
    }

    pub fn try_replace_with(&mut self, indices: Vec<impl Into<TracingIndex>>, f: impl FnOnce(Vec<X>) -> X) -> Result<TimedIndex, IndexError> {
        self.try_replace_choose(indices, |indices| indices[0], f)
    }

    pub fn try_replace_at_last_with(&mut self, indices: Vec<impl Into<TracingIndex>>, f: impl FnOnce(Vec<X>) -> X) -> Result<TimedIndex, IndexError> {
        self.try_replace_choose(indices, |indices| *indices.last().unwrap(), f)
    }

    fn try_replace_choose(
        &mut self,
        indices: Vec<impl Into<TracingIndex>>,
        choose: impl FnOnce(Vec<usize>) -> usize,
        f: impl FnOnce(Vec<X>) -> X,
    ) -> Result<TimedIndex, IndexError> {

        let (interleaved_positions, removed) = self.try_remove_all(indices)?;

        let chosen = choose(interleaved_positions);

        let removed =
        removed
            .into_iter()
            .map(|index|
                self.get(index)
                    .ok()
                    .cloned()
                    .unwrap()
            )
            .collect();

        let val = f(removed);
        let birth = self.pseudotime();

        self.mem.push(Trace { val, birth });
        let tracing_index = self.last_obj_index();

        self.snapshots
            .last_mut()
            .unwrap()
            .insert(chosen, tracing_index);

        let pos = chosen;
        let pseudotime = birth;

        Ok(TimedIndex { pos, pseudotime })
    }
}

// IMPL: Accessing
//
impl<X> TracingVec<X> {
    pub fn oldest(&self) -> Vec<&X> {
        self.snapshots
            .first()
            .unwrap()
            .iter()
            .map(|&pos| &self.mem[pos].val)
            .collect()
    }


    pub fn latest(&self) -> Vec<&X> {
        self.snapshots
            .last()
            .unwrap()
            .iter()
            .map(|&pos| &self.mem[pos].val)
            .collect()
    }

    pub fn latest_mut(&mut self) -> Vec<&mut X> {
        let mut ret = vec![];

        let mut mem_mut: Vec<_> =
        self.mem
            .iter_mut()
            .map(Some)
            .collect();

        for &pos in self.snapshots
            .last()
            .unwrap() {

            // NOTE: Since otherwise, we can't prove that we don't borrow mutably some indices
            // twice.
            //
            let val = &mut mem_mut.remove(pos).unwrap().val;

            // NOTE: Not to mess up other indices by the removal.
            //
            mem_mut.insert(pos, None);

            ret.push(val);
        }

        ret
    }


    pub fn first_index(&self) -> TimedIndex {
        self.try_first_index().unwrap()
    }

    pub fn try_first_index(&self) -> Option<TimedIndex> {
        let v = self.latest();

        if v.is_empty() {
            None

        } else {
            let pseudotime = self.pseudotime();
            let pos = 0;

            Some(TimedIndex { pseudotime, pos })
        }
    }


    pub fn last_index(&self) -> TimedIndex {
        self.try_last_index().unwrap()
    }

    pub fn try_last_index(&self) -> Option<TimedIndex> {
        let pseudotime = self.pseudotime();

        let len = self.latest_snapshot().len();

        if len == 0 {
            None

        } else {
            Some(TimedIndex { pseudotime, pos: len - 1 })
        }
    }


    pub fn len(&self) -> usize {
        self.latest().len()
    }

    pub fn is_empty(&self) -> bool {
        self.latest().is_empty()
    }


    /// Checks if the index points to a valid location within the vector.
    ///
    pub fn contains(&self, index: impl Into<TracingIndex>) -> bool {
        self.into_timeless(index).is_ok()
    }

    /// Checks whether or not the value that the index points to is still alive in the latest
    /// snapshot. If the index isn't valid, this returns `false`.
    ///
    pub fn is_alive(&self, index: impl Into<TracingIndex>) -> bool {
        self.into_timeless(index)
            .map(|TimelessIndex { pos }|
                self.snapshots
                    .last()
                    .unwrap()
                    .contains(&pos)
            )
            .unwrap_or(false)
    }

    pub fn get(&self, index: impl Into<TracingIndex>) -> Result<&X, IndexError> {
        let index = self.into_timeless(index)?;
        let trace = &self.mem[index.pos];

        Ok(&trace.val)
    }

    pub fn get_mut(&mut self, index: impl Into<TracingIndex>) -> Result<&mut X, IndexError> {
        let index = self.into_timeless(index)?;
        let trace = &mut self.mem[index.pos];

        Ok(&mut trace.val)
    }
}

// IMPL: Iteration
//
impl<X> TracingVec<X> {
    fn refs(&self) -> impl Iterator<Item = usize> {
        self.snapshots
            .last()
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn iter(&self) -> impl Iterator<Item = &X> + DoubleEndedIterator + ExactSizeIterator + Clone {
        self.latest()
            .into_iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut X> + DoubleEndedIterator + ExactSizeIterator {
        self.latest_mut()
            .into_iter()
    }


    pub fn indices(&self) -> impl Iterator<Item = TimedIndex> + DoubleEndedIterator + ExactSizeIterator + Clone {
        let pseudotime = self.pseudotime();

        self.snapshots
            .last()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(pos, _)| pos)
            .map(move |pos| TimedIndex { pos, pseudotime })
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn iter_indices(&self) -> impl Iterator<Item = (TimedIndex, &X)> + DoubleEndedIterator + ExactSizeIterator + Clone {
        self.indices().zip(self.iter())
    }

    pub fn iter_mut_indices(&mut self) -> impl Iterator<Item = (TimedIndex, &mut X)> + DoubleEndedIterator + ExactSizeIterator {
        self.indices().zip(self.iter_mut())
    }


    pub fn timeless_indices(&self) -> impl Iterator<Item = TimelessIndex> + DoubleEndedIterator + ExactSizeIterator + Clone {
        self.indices()
            .map(|timed| self.into_timeless(timed).unwrap())
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn iter_timeless_indices(&self) -> impl Iterator<Item = (TimelessIndex, &X)> + DoubleEndedIterator {
        self.timeless_indices().zip(self.iter())
    }

    pub fn iter_mut_timeless_indices(&mut self) -> impl Iterator<Item = (TimelessIndex, &mut X)> + DoubleEndedIterator {
        self.timeless_indices().zip(self.iter_mut())
    }
}

// IMPL: Utils
//
impl<X> TracingVec<X> {
    fn pseudotime(&self) -> usize {
        self.snapshots.len() - 1
    }


    fn last_obj_index(&self) -> usize {
        self.mem.len() - 1
    }

    fn val_location(&self, index: TimedIndex) -> Result<usize, IndexError> {
        let snapshot =
        self.snapshots
            .get(index.pseudotime)
            .ok_or(IndexError::VersionDoesNotExist(index))?;

        snapshot.get(index.pos)
            .copied()
            .ok_or(IndexError::IndexOutOfBounds(index))
    }


    pub fn into_timeless(&self, index: impl Into<TracingIndex>) -> Result<TimelessIndex, IndexError> {
        match index.into() {
            TracingIndex::Timed(index) =>
                Ok(TimelessIndex { pos: self.val_location(index)? }),

            TracingIndex::Timeless(index) =>
                if self.mem.len() > index.pos {
                    Ok(index)

                } else {
                    Err(IndexError::DataDoesNotExist(index))
                },
        }
    }

    pub fn into_timed(&self, index: impl Into<TracingIndex>) -> Result<TimedIndex, IndexError> {
        Ok(TimedIndex {
            pseudotime: self.pseudotime(),

            pos: self.latest_index(index)?,
        })
    }


    pub fn is_before(&self, before: impl Into<TracingIndex>, after: impl Into<TracingIndex>) -> Result<bool, IndexError> {
        let before = self.latest_index(before)?;
        let after = self.latest_index(after)?;

        Ok(before <= after)
    }

    pub fn indices_eq(&self, a: impl Into<TracingIndex>, b: impl Into<TracingIndex>) -> Result<bool, IndexError> {
        Ok(self.into_timeless(a)? == self.into_timeless(b)?)
    }


    fn latest_snapshot(&self) -> &[usize] {
        self.snapshots
            .last()
            .unwrap()
    }

    fn latest_snapshot_mut(&mut self) -> &mut Vec<usize> {
        self.snapshots.last_mut().unwrap()
    }


    fn new_snapshot(&mut self) -> &mut Vec<usize> {
        let last_snapshot_copy = self
            .snapshots
            .last()
            .unwrap()
            .clone();

        self.snapshots.push(last_snapshot_copy);

        self.latest_snapshot_mut()
    }

    fn latest_index(&self, index: impl Into<TracingIndex>) -> Result<usize, IndexError> {
        let index = self.into_timeless(index)?;

        if !self.is_alive(index) {
            return Err(IndexError::DataAlreadyDead(index));
        }

        Ok(
            self.refs()
                .enumerate()
                .filter(|(_, rf)| *rf == index.pos)
                .next()
                .unwrap()
                .0
        )
    }
}
