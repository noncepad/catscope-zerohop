pub mod server;
pub use server::capacity_serve;
pub mod proto {
    tonic::include_proto!("relay");
}
