package me.ztd.block.installer;

import java.io.DataInputStream;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.lang.instrument.Instrumentation;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URL;
import java.net.URLConnection;
import java.net.URLStreamHandler;
import java.net.URLStreamHandlerFactory;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.HashMap;
import java.util.Map;

/** Read-only inputs for unmodified installer processors. Network IO belongs to Block. */
public final class InstallerCache {
    private InstallerCache() {}

    public static void premain(String config, Instrumentation unused) throws IOException {
        final Map<String, Path> inputs = new HashMap<String, Path>();
        try (DataInputStream stream = new DataInputStream(new FileInputStream(config))) {
            int count = stream.readInt();
            if (count < 0 || count > 32) throw new IOException("Invalid installer input count");
            for (int index = 0; index < count; index++) {
                String url = readString(stream);
                Path path = Paths.get(readString(stream));
                if (!Files.isRegularFile(path) || inputs.put(url, path) != null)
                    throw new IOException("Invalid or duplicate installer input");
            }
            if (stream.read() != -1) throw new IOException("Trailing installer input data");
        }
        URL.setURLStreamHandlerFactory(new URLStreamHandlerFactory() {
            @Override public URLStreamHandler createURLStreamHandler(String protocol) {
                if (!protocol.equals("http") && !protocol.equals("https")) return null;
                return new URLStreamHandler() {
                    @Override protected URLConnection openConnection(URL url) throws IOException {
                        Path path = inputs.get(url.toExternalForm());
                        if (path == null) throw new IOException("Installer requested an unprepared network resource");
                        return new CachedConnection(url, path);
                    }
                    @Override protected URLConnection openConnection(URL url, Proxy proxy) throws IOException {
                        return openConnection(url);
                    }
                };
            }
        });
    }

    private static String readString(DataInputStream stream) throws IOException {
        int length = stream.readInt();
        if (length < 0 || length > 1024 * 1024) throw new IOException("Invalid installer input length");
        byte[] bytes = new byte[length];
        stream.readFully(bytes);
        return new String(bytes, StandardCharsets.UTF_8);
    }

    private static final class CachedConnection extends HttpURLConnection {
        private final Path path;
        CachedConnection(URL url, Path path) { super(url); this.path = path; }
        @Override public void connect() throws IOException {
            if (!Files.isRegularFile(path)) throw new IOException("Installer input is missing");
            connected = true;
        }
        @Override public void disconnect() { connected = false; }
        @Override public boolean usingProxy() { return false; }
        @Override public int getResponseCode() throws IOException { connect(); return HTTP_OK; }
        @Override public InputStream getInputStream() throws IOException { connect(); return Files.newInputStream(path); }
        @Override public long getContentLengthLong() {
            try { return Files.size(path); } catch (IOException error) { return -1; }
        }
        @Override public int getContentLength() {
            long length = getContentLengthLong();
            return length > Integer.MAX_VALUE ? -1 : (int) length;
        }
        @Override public String getHeaderField(String name) {
            return "Content-Length".equalsIgnoreCase(name) ? Long.toString(getContentLengthLong()) : null;
        }
    }
}
