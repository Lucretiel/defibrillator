use std::{
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use futures::FutureExt;
use tokio::task::{JoinError, JoinHandle};

#[derive(Debug)]
pub struct ScopedTask<T> {
    task: JoinHandle<T>,
}

impl<T> ScopedTask<T> {
    pub fn new(task: JoinHandle<T>) -> Self {
        Self { task }
    }
}

impl<T> From<JoinHandle<T>> for ScopedTask<T> {
    fn from(task: JoinHandle<T>) -> Self {
        Self::new(task)
    }
}

impl<T> Deref for ScopedTask<T> {
    type Target = JoinHandle<T>;

    fn deref(&self) -> &Self::Target {
        &self.task
    }
}

impl<T> DerefMut for ScopedTask<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.task
    }
}

impl<T> Future for ScopedTask<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.task.poll_unpin(cx)
    }
}

impl<T> Drop for ScopedTask<T> {
    fn drop(&mut self) {
        self.task.abort()
    }
}
