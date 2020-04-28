use tokio;
use tokio::io::BufReader;
use tokio::prelude::*;

pub struct Stream<R, W> {
    pub rh: ReadHalf<R>,
    pub wh: WriteHalf<W>,
}

impl<R, W> Stream<R, W>
where
    R: AsyncRead + std::marker::Unpin,
    W: AsyncWrite + std::marker::Unpin,
{
    pub fn new_from_halfs(rh: ReadHalf<R>, wh: WriteHalf<W>) -> Self {
        Self { rh, wh }
    }
}

impl<T> Stream<io::ReadHalf<T>, io::WriteHalf<T>>
where
    T: AsyncWrite + AsyncRead,
{
    pub fn new(stream: T) -> Stream<io::ReadHalf<T>, io::WriteHalf<T>> {
        let (stream_rh, stream_wh) = io::split(stream);
        let rh = ReadHalf::new(stream_rh);
        let wh = WriteHalf::new(stream_wh);
        Stream::new_from_halfs(rh, wh)
    }
}

pub struct ReadHalf<R> {
    inner: BufReader<R>,
}

impl<R> ReadHalf<R>
where
    R: AsyncRead + std::marker::Unpin,
{
    pub fn new(inner: R) -> Self {
        Self {
            inner: BufReader::with_capacity(1024, inner),
        }
    }

    pub async fn http_header(&mut self) -> Result<Vec<String>, io::Error> {
        let mut res: Vec<String> = Vec::new();
        loop {
            let mut line = String::new();
            // TODO: sta ako ovdje nikada nista ne posalje blokira mi thread !!!
            match self.inner.read_line(&mut line).await? {
                0 => break, // eof
                2 => break, // empty line \r\n = end of header line
                _n => res.push(line.trim().to_owned()),
            }
        }
        Ok(res)
    }

    #[allow(dead_code)]
    async fn read_to_end(&mut self) -> Result<Vec<u8>, io::Error> {
        let mut res = vec![];
        self.inner.read_to_end(&mut res).await?;
        Ok(res)
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read_exact(buf).await
    }
}

pub struct WriteHalf<W> {
    inner: W,
}

impl<W> WriteHalf<W>
where
    W: AsyncWrite + std::marker::Unpin,
{
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub async fn write(&mut self, mut buf: &[u8]) -> Result<(), io::Error> {
        loop {
            let n = self.inner.write(buf).await?;
            if n == buf.len() {
                break;
            }
            // resize, remove written leave unwritten and try again
            buf = &buf[n..];
        }
        Ok(())
    }
}
