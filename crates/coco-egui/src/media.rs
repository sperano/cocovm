//! Everything the user can plug into, mount in, or eject from the emulated
//! machine, split by device family. All of it is `impl CocoApp` methods,
//! driven from the status-bar menus (`chrome::menus`) and
//! from the manager's launch path (`launch`).

mod banked_rompak;
mod cart;
mod cart_mpi;
pub(crate) mod disk;
pub(crate) mod drivewire;
mod printer;
mod tape;
