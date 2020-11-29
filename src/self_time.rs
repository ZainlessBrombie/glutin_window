use std::any::Any;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct SelfTime<'a> {
    alive: LinkList<'a>,
}

impl<'a> Default for SelfTime<'a> {
    fn default() -> Self {
        SelfTime::new()
    }
}

impl<'a> SelfTime<'a> {
    pub fn new() -> SelfTime<'a> {
        SelfTime {
            alive: LinkList::None,
        }
    }

    pub fn push<T: 'static>(&mut self, el: T) -> &mut T {
        let mut cur = LinkList::None;
        std::mem::swap(&mut self.alive, &mut cur);
        let mut next = LinkList::Element(Box::new((cur, Box::new(el))));
        std::mem::swap(&mut self.alive, &mut next);

        if let LinkList::Element(b) = &mut self.alive {
            let x: &mut T = ((*b).deref_mut().1).downcast_mut().unwrap();
            return x;
        }
        unreachable!()
    }
}

pub(crate) type LTup<'a> = (LinkList<'a>, Box<dyn Any>);

pub(crate) enum LinkList<'a> {
    None,
    Element(Box<LTup<'a>>),
    Never(PhantomData<&'a ()>),
}
