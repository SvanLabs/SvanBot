//! The betting tree of the rest of one street, heads up, from our turn.
//!
//! Chips are counted from the start of the re-solve: `base` is the pot without the bet we face,
//! and each player's `added` is what they put in since, so the villain starts with the bet.
//! Not facing a bet, a player may check, bet half the pot, bet the pot or go all in. Facing one,
//! they may fold, call, raise the pot or go all in, up to [`MAX_BETS`] bets and raises a street.
//! A call, or a check back, ends the street.

/// Bets and raises a street may hold.
pub const MAX_BETS: u32 = 3;

/// An action in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abstract {
    Fold,
    Check,
    Call,
    /// A bet or raise that adds this many chips.
    Bet(u32),
}

/// A node of the tree.
#[derive(Clone, Debug)]
pub enum Node {
    /// `player` acts: 0 is us, 1 the villain.
    Act {
        player: usize,
        actions: Vec<Abstract>,
        children: Vec<usize>,
    },
    /// `folder` folded.
    Fold { folder: usize, added: [u32; 2] },
    /// The street ends with both in.
    Leaf { added: [u32; 2] },
}

/// The tree, root first.
#[derive(Clone, Debug)]
pub struct Tree {
    pub base: u32,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
struct State {
    player: usize,
    added: [u32; 2],
    bets: u32,
    /// The player before checked.
    checked: bool,
}

impl Tree {
    /// The tree from our turn: `base` in the pot, facing `call`, with `stack` behind. The villain
    /// is taken to cover us.
    pub fn new(base: u32, call: u32, stack: u32) -> Self {
        let mut tree = Self {
            base,
            nodes: Vec::new(),
        };
        let caps = [stack, stack.saturating_add(call)];
        tree.grow(
            State {
                player: 0,
                added: [0, call],
                bets: u32::from(call > 0),
                checked: false,
            },
            caps,
        );
        tree
    }

    fn grow(&mut self, state: State, caps: [u32; 2]) -> usize {
        let at = self.nodes.len();
        self.nodes.push(Node::Leaf { added: state.added });
        let me = state.player;
        let other = 1 - me;
        let behind = caps[me].saturating_sub(state.added[me]);
        let owed = state.added[other].saturating_sub(state.added[me]);
        let pot = self.base + state.added[0] + state.added[1];
        let mut actions = Vec::new();
        if owed > 0 {
            actions.push(Abstract::Fold);
            actions.push(Abstract::Call);
            if state.bets < MAX_BETS && behind > owed {
                let raise = owed + (pot + owed);
                if raise < behind {
                    actions.push(Abstract::Bet(raise));
                }
                actions.push(Abstract::Bet(behind));
            }
        } else {
            actions.push(Abstract::Check);
            if state.bets < MAX_BETS && behind > 0 {
                for size in [pot / 2, pot] {
                    if size > 0 && size < behind && !actions.contains(&Abstract::Bet(size)) {
                        actions.push(Abstract::Bet(size));
                    }
                }
                actions.push(Abstract::Bet(behind));
            }
        }
        let mut children = Vec::with_capacity(actions.len());
        for &action in &actions {
            let child = match action {
                Abstract::Fold => {
                    let node = self.nodes.len();
                    self.nodes.push(Node::Fold {
                        folder: me,
                        added: state.added,
                    });
                    node
                }
                Abstract::Call => {
                    let mut added = state.added;
                    added[me] += owed.min(behind);
                    let node = self.nodes.len();
                    self.nodes.push(Node::Leaf { added });
                    node
                }
                Abstract::Check if state.checked => {
                    let node = self.nodes.len();
                    self.nodes.push(Node::Leaf { added: state.added });
                    node
                }
                Abstract::Check => self.grow(
                    State {
                        player: other,
                        checked: true,
                        ..state
                    },
                    caps,
                ),
                Abstract::Bet(chips) => {
                    let mut added = state.added;
                    added[me] += chips;
                    self.grow(
                        State {
                            player: other,
                            added,
                            bets: state.bets + 1,
                            checked: false,
                        },
                        caps,
                    )
                }
            };
            children.push(child);
        }
        self.nodes[at] = Node::Act {
            player: me,
            actions,
            children,
        };
        at
    }
}
