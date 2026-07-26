//! Everything the user can plug into, mount in, or eject from the emulated
//! machine, split by device family. All of it is `impl CocoApp` methods,
//! driven from the Machine menu (`chrome::menu_bar`) and from the CLI
//! (`boot`) and manager (`launch`) boot paths.

mod cart;
mod disk;
mod drivewire;
mod printer;
mod tape;
