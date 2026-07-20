#!/usr/bin/perl
#
# Start folding on the local Folding@Home v8 client.
#
# A v8 client that is linked to an account (account-token) comes up *paused* and
# stays that way until something sends it a "fold" command. Nothing in config.xml
# can change this: the only control surface is the client's local WebSocket API
# (the same one Web Control drives). This script performs the WebSocket handshake
# and sends a single {"cmd":"state","state":"fold"} frame, which flips the default
# resource group's `paused` flag to false and makes it request work units.
#
# Sending "fold" when the client is already folding is a harmless no-op, so this
# is safe to run on every start.
#
# Uses only core Perl modules so the image needs no extra packages.
#
# Usage: fold.pl [host:port]   (default 127.0.0.1:7396)

use strict;
use warnings;
use IO::Socket::INET;

# Minimal base64 encoder so we depend only on perl-base (MIME::Base64 is not in
# the slim image). Used just for the Sec-WebSocket-Key nonce.
sub b64 {
    my @c = ('A' .. 'Z', 'a' .. 'z', '0' .. '9', '+', '/');
    my $out = '';
    for (my $i = 0; $i < length($_[0]); $i += 3) {
        my @g = unpack('C*', substr($_[0], $i, 3));
        my $n = (($g[0] // 0) << 16) | (($g[1] // 0) << 8) | ($g[2] // 0);
        $out .= $c[($n >> 18) & 63] . $c[($n >> 12) & 63]
              . (defined $g[1] ? $c[($n >> 6) & 63] : '=')
              . (defined $g[2] ? $c[$n & 63] : '=');
    }
    return $out;
}

my $target = $ARGV[0] || '127.0.0.1:7396';
my ($host, $port) = split /:/, $target;
$port ||= 7396;
my $path = '/api/websocket';

my $sock = IO::Socket::INET->new(
    PeerAddr => $host,
    PeerPort => $port,
    Proto    => 'tcp',
    Timeout  => 5,
) or die "fold: cannot connect to $host:$port: $!\n";
$sock->autoflush(1);

# --- WebSocket opening handshake (RFC 6455) ---
my $key = b64(pack('N4', map { int(rand(2**32)) } 1 .. 4));
print $sock
    "GET $path HTTP/1.1\r\n"
  . "Host: $host:$port\r\n"
  . "Upgrade: websocket\r\n"
  . "Connection: Upgrade\r\n"
  . "Sec-WebSocket-Key: $key\r\n"
  . "Sec-WebSocket-Version: 13\r\n"
  . "Origin: http://$host:$port\r\n"
  . "\r\n";

# Read response headers up to the blank line.
my $resp = '';
local $/ = "\r\n";
while (my $line = <$sock>) {
    $resp .= $line;
    last if $line eq "\r\n";
}
die "fold: handshake failed: $resp\n" unless $resp =~ m{^HTTP/1\.\d\s+101}i;

# --- Send one masked text frame with the fold command ---
my @t = gmtime;
my $ts = sprintf('%04d-%02d-%02dT%02d:%02d:%02d.000Z',
    $t[5] + 1900, $t[4] + 1, @t[3, 2, 1, 0]);
my $msg = qq({"state":"fold","group":"","cmd":"state","time":"$ts"});

my $len = length $msg;
my $frame = pack('C', 0x81);          # FIN + text opcode
# Client frames MUST be masked; set the mask bit and length.
if ($len < 126) {
    $frame .= pack('C', 0x80 | $len);
} elsif ($len < 65536) {
    $frame .= pack('C', 0x80 | 126) . pack('n', $len);
} else {
    $frame .= pack('C', 0x80 | 127) . pack('Q>', $len);
}
my @mask = map { int(rand(256)) } 1 .. 4;
$frame .= pack('C4', @mask);
my @bytes = unpack('C*', $msg);
$frame .= pack('C*', map { $bytes[$_] ^ $mask[$_ % 4] } 0 .. $#bytes);

print $sock $frame;

# Give the client a moment to process before we drop the connection.
select(undef, undef, undef, 0.5);
close $sock;
print "fold: sent state:fold to $host:$port\n";
